//! Writing record data: value encoding, record-id sets, and rebuilding a
//! file with new table contents. See `docs/FORMAT.md` §13.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{self, Records};
use crate::write::{self, Entry, Item};
use crate::{Error, File, XOR, int_path};

/// Values longer than this many bytes move to a sub-path, in chunks of
/// this size.
const INLINE_MAX: usize = 1000;

/// Path element spelling of an integer (inverse of [`crate::path_int`]).
pub fn path_elem(n: u32) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else {
        let v = n - 0x80;
        vec![0x80 | (v >> 8) as u8, v as u8]
    }
}

/// FileMaker's length-prefixed integer: `01 28` = 40, `02 01 2C` = 300.
pub fn len_int(n: u64) -> Vec<u8> {
    let b = n.to_be_bytes();
    let skip = b.iter().take_while(|x| **x == 0).count().min(7);
    let mut out = vec![(8 - skip) as u8];
    out.extend_from_slice(&b[skip..]);
    out
}

/// Inverse of [`len_int`].
pub fn read_len_int(b: &[u8]) -> Option<u64> {
    let (n, rest) = b.split_first()?;
    let v = rest.get(..*n as usize)?;
    Some(v.iter().fold(0, |a, x| (a << 8) | *x as u64))
}

/// Text as FileMaker stores it: SCSU, XOR-ed with 0x5A.
pub fn encode_text(s: &str) -> Vec<u8> {
    crate::scsu::encode(s)
        .into_iter()
        .map(|b| b ^ XOR)
        .collect()
}

/// Entries storing `value` in field `field` of the record at path `rec`.
/// A value is inline (key = field id) when its SCSU form has one byte per
/// UTF-16 unit and at most 1000 bytes. Otherwise it moves to the sub-path
/// `[rec].[field]`: key 0 = byte count, UTF-16 count (u32 BE each, plus a
/// u32 4 for values over 1000 bytes), keys 1… = 1000-byte chunks.
pub fn value_entries(rec: &[Vec<u8>], field: u32, value: &str) -> Vec<Entry> {
    let b = encode_text(value);
    let units = value.encode_utf16().count();
    let entry = |path: Vec<Vec<u8>>, key, data: &[u8]| Entry {
        path,
        item: write::encode(&Item::Kv {
            key,
            data: data.to_vec(),
        }),
    };
    if b.len() == units && b.len() <= INLINE_MAX {
        return vec![entry(rec.to_vec(), field, &b)];
    }
    let mut path = rec.to_vec();
    path.push(path_elem(field));
    let mut head = (b.len() as u32).to_be_bytes().to_vec();
    head.extend_from_slice(&(units as u32).to_be_bytes());
    if b.len() > INLINE_MAX {
        // ponytail: always 4 in our samples; its meaning is unknown.
        head.extend_from_slice(&4u32.to_be_bytes());
    }
    let mut out = vec![
        Entry {
            path: path.clone(),
            item: vec![0x80],
        },
        entry(path.clone(), 0, &head),
    ];
    for (i, c) in b.chunks(INLINE_MAX).enumerate() {
        out.push(entry(path.clone(), i as u32 + 1, c));
    }
    out
}

/// Item of a set of record ids, as in `[T].[13]` (all records) and the
/// postings of a field index: one id below 256 is a 1-byte key with an
/// empty value (`00 id`); otherwise key 0 holds a bitmap of u32 BE words,
/// word i bit j (from the least significant) = id 32i + j.
/// ponytail: a bitmap over 255 bytes (ids ≥ 2040) uses the long Kv form;
/// FileMaker may split big sets over keys 1…, never observed.
pub fn id_set(ids: &BTreeSet<u32>) -> Option<Vec<u8>> {
    let max = *ids.last()?;
    if ids.len() == 1 && max < 0x100 {
        return Some(vec![0x00, max as u8]);
    }
    let mut words = vec![0u32; max as usize / 32 + 1];
    for id in ids {
        words[*id as usize / 32] |= 1 << (id % 32);
    }
    let data = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    Some(write::encode(&Item::Kv { key: 0, data }))
}

/// Inverse of [`id_set`] for one chunk (as decoded by the reader).
pub fn read_id_set(c: &crate::Chunk) -> BTreeSet<u32> {
    match c {
        crate::Chunk::Data([id]) => BTreeSet::from([*id as u32]),
        crate::Chunk::Kv { key: 0, data } => data
            .chunks(4)
            .enumerate()
            .flat_map(|(i, w)| {
                let w = u32::from_be_bytes([w[0], w[1], w[2], w[3]]);
                (0..32)
                    .filter(move |j| w & (1 << j) != 0)
                    .map(move |j| 32 * i as u32 + j)
            })
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Highest record id a table has ever used (`[T]` key 0; FileMaker never
/// reuses ids of deleted records), 0 if none.
pub fn last_record_id(file: &File, table: u32) -> Result<u32, Error> {
    let mut out = 0;
    file.walk(|path, c| {
        if let (Some([t]), crate::Chunk::Kv { key: 0, data }) = (int_path(path).as_deref(), c)
            && *t == table
        {
            out = read_len_int(data).unwrap_or(0) as u32;
        }
    })?;
    Ok(out)
}

fn kv(path: Vec<Vec<u8>>, key: u32, data: Vec<u8>) -> Entry {
    Entry {
        path,
        item: write::encode(&Item::Kv { key, data }),
    }
}

/// Key of an item read back as an integer (1-byte or path-int spelling).
fn item_int_key(item: &[u8]) -> Option<u32> {
    match item.first()? {
        0x00..=0x07 => item.get(1).map(|k| *k as u32),
        0x08..=0x0F => crate::path_int(item.get(1..3)?),
        _ => None,
    }
}

/// Rebuilds `file` with the given records for some of its tables (table id
/// → all its records). Records whose values are unchanged keep their exact
/// bytes; changed and new ones are re-encoded, deleted ones removed. Also
/// updated: the record-id set `[T].[13]`, the last record id `[T]` key 0,
/// and the table's change counter (`[3].[16].[5].[T]` key 251). The index
/// `[T].[1].[F]` of every field whose values changed is removed and the
/// field marked unindexed: FileMaker trusts a stored index (relationships,
/// value lists, finds) but builds a missing one on demand for a field with
/// automatic indexing (FORMAT.md §13).
pub fn save_records(file: &File, data: &BTreeMap<u32, Records>) -> Result<Vec<u8>, Error> {
    let tables = schema::tables(file)?;
    let mut entries = write::entries(file)?;
    for (&t, new) in data {
        let Some(table) = tables.iter().find(|x| x.id == t) else {
            continue;
        };
        let old = schema::records(file, table)?;
        // Empty values are not stored.
        let new: Records = new
            .iter()
            .map(|(r, v)| {
                let v = v.iter().filter(|(_, s)| !s.is_empty());
                (*r, v.map(|(f, s)| (*f, s.clone())).collect())
            })
            .collect();
        let changed: BTreeSet<u32> = old
            .keys()
            .chain(new.keys())
            .copied()
            .filter(|r| old.get(r) != new.get(r))
            .collect();
        if changed.is_empty() {
            continue;
        }
        // Fields whose value changed in some record: their index is dropped.
        // Empty values are indexed too, so a new or deleted record touches all.
        let added_or_deleted = changed
            .iter()
            .any(|r| old.contains_key(r) != new.contains_key(r));
        let fields: BTreeSet<u32> = if added_or_deleted {
            table.fields.keys().copied().collect()
        } else {
            changed
                .iter()
                .flat_map(|r| {
                    let (o, n) = (old.get(r), new.get(r));
                    let keys = o.into_iter().chain(n).flat_map(|v| v.keys());
                    keys.filter(move |f| o.and_then(|v| v.get(f)) != n.and_then(|v| v.get(f)))
                        .copied()
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        // ponytail: with automatic indexing off, the stale index is kept;
        // FileMaker would not rebuild it.
        let fields: BTreeSet<u32> = fields
            .into_iter()
            .filter(|f| table.fields.get(f).is_some_and(|d| d.auto_index()))
            .collect();
        let last = last_record_id(file, t)?.max(new.keys().copied().max().unwrap_or(0));
        let tp = path_elem(t);
        // Drop what is regenerated; keep non-field keys (e.g. 252) of edited records.
        let mut kept_meta: BTreeMap<u32, Vec<Entry>> = BTreeMap::new();
        entries.retain(|e| {
            // Compare spelled elements: a TO reference `D0 00 02` also reads as 130.
            let int = |i: usize| e.path.get(i).and_then(|x| crate::path_int(x));
            let key = item_int_key(&e.item);
            if e.path.first() == Some(&tp) {
                return match (e.path.len(), int(1)) {
                    // [T] key 0: last record id.
                    (1, _) => key != Some(0),
                    // [T].[1].[F]…: index of a changed field.
                    (2.., Some(1)) => !int(2).is_some_and(|f| fields.contains(&f)),
                    // [T].[5].[R]…: a changed record.
                    (3.., Some(5)) if int(2).is_some_and(|r| changed.contains(&r)) => {
                        let r = int(2).unwrap();
                        let meta = e.path.len() == 3
                            && key.is_some_and(|k| !table.fields.contains_key(&k));
                        if meta && new.contains_key(&r) {
                            kept_meta.entry(r).or_default().push(e.clone());
                        }
                        false
                    }
                    // [T].[13]: record-id set.
                    (2, Some(13)) => e.item.first() == Some(&0x80),
                    _ => true,
                };
            }
            let counter = e.path.len() == 4
                && e.path[..3] == [[3], [16], [5]]
                && e.path[3] == tp
                && key == Some(251);
            !counter
        });
        // The fields whose index was dropped are marked unindexed, so that
        // FileMaker builds the index again when it needs it (and marks the
        // field indexed again), exactly as for a never-indexed field.
        for e in entries.iter_mut() {
            let field_def = e.path.len() == 4
                && e.path[0] == tp
                && e.path[1..3] == [[3], [5]]
                && crate::path_int(&e.path[3]).is_some_and(|f| fields.contains(&f));
            if let (true, Ok(chunks)) = (field_def, crate::chunk::parse(&e.item))
                && let [crate::Chunk::Kv { key: 2, data }] = chunks.as_slice()
                && data.len() > 8
            {
                let mut data = data.to_vec();
                data[8] &= !0b1010_1000;
                e.item = write::encode(&Item::Kv { key: 2, data });
            }
        }
        // Change counter and last record id.
        let counter = table_counter(file, t)?.wrapping_add(1);
        entries.push(kv(
            [3, 16, 5]
                .iter()
                .map(|x| vec![*x])
                .chain([tp.clone()])
                .collect(),
            251,
            counter.to_be_bytes().to_vec(),
        ));
        entries.push(kv(vec![tp.clone()], 0, len_int(last as u64)));
        if !entries
            .iter()
            .any(|e| e.path == [tp.clone()] && e.item == [0x80])
        {
            entries.push(Entry {
                path: vec![tp.clone()],
                item: vec![0x80],
            });
        }
        // Record-id set.
        let p13 = vec![tp.clone(), vec![13]];
        if let Some(item) = id_set(&new.keys().copied().collect()) {
            if !entries.iter().any(|e| e.path == p13 && e.item == [0x80]) {
                entries.push(Entry {
                    path: p13.clone(),
                    item: vec![0x80],
                });
            }
            entries.push(Entry { path: p13, item });
        }
        // Changed and new records.
        for r in &changed {
            let Some(values) = new.get(r) else { continue };
            let rec = vec![tp.clone(), vec![5], path_elem(*r)];
            entries.push(Entry {
                path: rec.clone(),
                item: vec![0x80],
            });
            for (f, v) in values {
                if !v.is_empty() {
                    entries.extend(value_entries(&rec, *f, v));
                }
            }
            entries.extend(kept_meta.remove(r).unwrap_or_default());
        }
    }
    write::sort(&mut entries);
    Ok(write::build(&file.bytes()[..crate::SECTOR], &entries))
}

/// `[3].[16].[5].[T]` key 251: incremented by FileMaker whenever the
/// table's records change (once per transaction).
fn table_counter(file: &File, table: u32) -> Result<u32, Error> {
    let mut out = 0;
    file.walk(|path, c| {
        if let (Some([3, 16, 5, t]), crate::Chunk::Kv { key: 251, data }) =
            (int_path(path).as_deref(), c)
            && *t == table
            && data.len() == 4
        {
            out = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_ints() {
        assert_eq!(len_int(40), [1, 0x28]);
        assert_eq!(len_int(300), [2, 1, 0x2C]);
        assert_eq!(len_int(0), [1, 0]);
        assert_eq!(read_len_int(&[2, 1, 0x2C]), Some(300));
    }

    #[test]
    fn id_sets_match_filemaker() {
        assert_eq!(id_set(&BTreeSet::from([1])).unwrap(), [0x00, 0x01]);
        assert_eq!(
            id_set(&BTreeSet::from([1, 2])).unwrap(),
            [0x03, 0x00, 0, 0, 0, 6]
        );
        let all: BTreeSet<u32> = (1..=40).collect();
        let item = id_set(&all).unwrap();
        assert_eq!(item, [0x05, 0x00, 0xFF, 0xFF, 0xFF, 0xFE, 0, 0, 1, 0xFF]);
        let c = crate::chunk::parse(&item).unwrap()[0];
        assert_eq!(read_id_set(&c), all);
    }

    #[test]
    fn values_inline_or_in_subpath() {
        let rec = vec![vec![0x80, 2], vec![5], vec![1]];
        // Seen in FileMaker: "P-001" inline as `06 06 05 …`.
        let e = value_entries(&rec, 6, "P-001");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].item, [0x06, 0x06, 0x05, 0x0A, 0x77, 0x6A, 0x6A, 0x6B]);
        // An em dash takes 2 bytes: sub-path with byte and character counts.
        let e = value_entries(&rec, 3, "Café crème — spécial");
        assert_eq!(e[1].item, [0x05, 0, 0, 0, 0, 0x15, 0, 0, 0, 0x14]);
        assert_eq!(e[2].item[..3], [0x06, 0x01, 0x15]);
        // Over 1000 bytes: chunks of 1000, and a third u32 in key 0.
        let e = value_entries(&rec, 6, &"p".repeat(1001));
        assert_eq!(
            e[1].item,
            [6, 0, 12, 0, 0, 3, 0xE9, 0, 0, 3, 0xE9, 0, 0, 0, 4]
        );
        assert_eq!(e.len(), 4);
    }
}
