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

/// A Kv item whose key is spelled `key` (1, 2 or 3 bytes: ops 01–07,
/// 09–0F, 10–17).
pub fn kv_raw(key: &[u8], data: &[u8]) -> Vec<u8> {
    let n = data.len();
    let (fixed, short, long) = match key.len() {
        1 => (0x01, 0x06, 0x07),
        2 => (0x09, 0x0E, 0x0F),
        _ => (0x11, 0x16, 0x17),
    };
    let three = key.len() == 3;
    let op = match n {
        0 if three => Some(0x10),
        1 => Some(fixed),
        2 | 4 | 6 | 8 => Some(fixed + (n / 2) as u8),
        _ => None,
    };
    let mut out = Vec::with_capacity(n + 6);
    match op {
        Some(op) => {
            out.push(op);
            out.extend_from_slice(key);
        }
        // `0E FF` reads as a 6-byte data chunk: such keys take the long form.
        None if n <= 255 && key.first() != Some(&0xFF) => {
            out.push(short);
            out.extend_from_slice(key);
            out.push(n as u8);
        }
        None => {
            out.push(long);
            out.extend_from_slice(key);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(data);
    out
}

/// Key bytes of an item read from a file, if it is a Kv with an integer
/// key (`00 k` included).
pub(crate) fn item_raw_key(item: &[u8]) -> Option<&[u8]> {
    match item.first()? {
        0x0E if item.get(1) == Some(&0xFF) => None,
        0x00..=0x07 => item.get(1..2),
        0x09..=0x0F => item.get(1..3),
        0x10..=0x17 => item.get(1..4),
        _ => None,
    }
}

/// Entries storing `value` at value key `key` ([`schema::rep_key`]: a field
/// id, or one of its repetitions) of the record at path `rec`.
/// A value is inline when its SCSU form has one byte per UTF-16 unit and
/// at most 1000 bytes. Otherwise it moves to the sub-path `[rec].[key]`:
/// key 0 = byte count, UTF-16 count (u32 BE each, plus a u32 4 for values
/// over 1000 bytes), keys 1… = 1000-byte chunks.
pub fn value_entries(rec: &[Vec<u8>], key: u32, value: &str) -> Vec<Entry> {
    let (field, rep) = schema::split_rep_key(key);
    let kb = schema::value_key_bytes(field, rep);
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
        return vec![Entry {
            path: rec.to_vec(),
            item: kv_raw(&kb, &b),
        }];
    }
    let mut path = rec.to_vec();
    path.push(kb);
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
                        .map(|k| schema::split_rep_key(*k).0)
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
            let value_key = item_raw_key(&e.item).and_then(|k| schema::value_key(k, table));
            if e.path.first() == Some(&tp) {
                return match (e.path.len(), int(1)) {
                    // [T] key 0: last record id.
                    (1, _) => key != Some(0),
                    // [T].[1].[F]…: index of a changed field.
                    (2.., Some(1)) => !int(2).is_some_and(|f| fields.contains(&f)),
                    // [T].[5].[R]…: a changed record.
                    (3.., Some(5)) if let Some(r) = int(2).filter(|r| changed.contains(r)) => {
                        let meta = e.path.len() == 3 && key.is_some() && value_key.is_none();
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
        // Changed and new records. The records node [T].[5] must exist:
        // FileMaker, finding it missing when it adds a record, starts it
        // afresh and loses the records under it.
        let p5 = vec![tp.clone(), vec![5]];
        if !new.is_empty() && !entries.iter().any(|e| e.path == p5 && e.item == [0x80]) {
            entries.push(Entry {
                path: p5,
                item: vec![0x80],
            });
        }
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

/// Keys (key, value) copied from a prototype object node.
type NodeKeys = Vec<(u32, Vec<u8>)>;

/// Integer path of an entry, `None` if an element is not an integer.
pub(crate) fn ipath(e: &Entry) -> Option<Vec<u32>> {
    e.path.iter().map(|p| crate::path_int(p)).collect()
}

/// (key, value) of a Kv entry; `00 k` (key k, empty value) included.
pub(crate) fn entry_kv(e: &Entry) -> Option<(u32, Vec<u8>)> {
    match crate::chunk::parse(&e.item).ok()?.first()? {
        crate::Chunk::Kv { key, data } => Some((*key, data.to_vec())),
        crate::Chunk::Data([k]) if e.item.first() == Some(&0) => Some((*k as u32, Vec::new())),
        _ => None,
    }
}

pub(crate) fn spelled(p: &[u32]) -> Vec<Vec<u8>> {
    p.iter().map(|x| path_elem(*x)).collect()
}

/// A Kv entry, `00 k` for an empty value as FileMaker writes it.
pub(crate) fn kv_entry(path: &[u32], key: u32, data: Vec<u8>) -> Entry {
    if data.is_empty() && key < 0x100 {
        return Entry {
            path: spelled(path),
            item: vec![0x00, key as u8],
        };
    }
    kv(spelled(path), key, data)
}

pub(crate) fn marker(path: &[u32]) -> Entry {
    Entry {
        path: spelled(path),
        item: vec![0x80],
    }
}

/// Calculation token stream of a text literal: `13 len text 00` (length in
/// UTF-16 units, text XOR-ed SCSU), as in button labels.
pub fn text_literal_tokens(s: &str) -> Vec<u8> {
    let units = s.encode_utf16().count().min(255) as u8;
    let mut out = vec![0x13, units];
    out.extend(encode_text(s));
    out.push(0);
    out
}

/// A random version-4 UUID for a new object (keys 216/217).
pub(crate) fn fresh_uuid(seed: u64) -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let state = std::collections::hash_map::RandomState::new();
    let mut out = Vec::new();
    for half in 0..2u64 {
        let mut h = state.build_hasher();
        h.write_u64(seed);
        h.write_u64(n);
        h.write_u64(half);
        out.extend_from_slice(&h.finish().to_be_bytes());
    }
    out[6] = (out[6] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

/// Object id referenced by a layout index value `00 len 01 id …`
/// (spatial index `[L].[10]`, tab order `[L].[21]`).
fn index_ref(v: &[u8]) -> Option<u32> {
    let n = *v.get(1)? as usize;
    match v.get(2..2 + n)? {
        [1, rest @ ..] => crate::path_int(rest),
        _ => None,
    }
}

/// The object list blob of layout `l`: key 7 of `[4].[5].[l]`, or keys 1…n
/// of `[4].[5].[l].[7]`.
fn object_blob(entries: &[Entry], l: u32) -> Vec<u8> {
    let mut parts: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for e in entries {
        match (ipath(e).as_deref(), entry_kv(e)) {
            (Some([4, 5, x]), Some((7, d))) if *x == l => {
                parts.insert(0, d);
            }
            (Some([4, 5, x, 7]), Some((k, d))) if *x == l && k >= 1 => {
                parts.insert(k, d);
            }
            _ => {}
        }
    }
    parts.into_values().flatten().collect()
}

/// Kinds that FileMaker lists in a layout's spatial index `[L].[10]`
/// (fields, buttons, tab controls, button bars; not text or shapes).
fn indexed_kind(kind: u64) -> bool {
    matches!(kind, 1 | 10 | 11 | 18)
}

/// Rebuilds `file` with edited layouts. Each layout of `layouts` that
/// differs from the file's is re-encoded: object list (patched messages,
/// new objects built on a prototype of the same kind, deleted ones
/// removed), per-object nodes `[4].[5].[L].[5].[obj]` (text, name, button
/// label, UUIDs), parts, name, table occurrence, width, the last object
/// id, and the object indexes `[L].[10]` and `[L].[21]`. Layouts not in
/// the file are skipped (writing new layouts is not supported yet).
pub fn save_layouts(file: &File, layouts: &[crate::layouts::Layout]) -> Result<Vec<u8>, Error> {
    use crate::layouts::{self as ly, LayoutObject};
    let old_all = ly::layouts(file)?;
    let mut entries = write::entries(file)?;
    // Prototype per (kind, style type): object message and node keys 7, 13.
    let mut protos: BTreeMap<(u64, u32), (u32, u64, Vec<u8>)> = BTreeMap::new();
    for l in &old_all {
        let blob = object_blob(&entries, l.id);
        let Some(m) = crate::pb::Msg::parse(&blob) else {
            continue;
        };
        for o in m.msgs(2) {
            let ty = crate::theme::ObjectStyle::parse(o.bytes(5).unwrap_or_default()).object_type;
            let key = (o.uint(1).unwrap_or(0), ty);
            protos
                .entry(key)
                .or_insert_with(|| (l.id, o.uint(2).unwrap_or(0), o.encode()));
        }
    }
    let node_keys = |entries: &[Entry], l: u32, id: u64| -> Vec<(u32, Vec<u8>)> {
        entries
            .iter()
            .filter(|e| ipath(e).as_deref() == Some(&[4, 5, l, 5, id as u32]))
            .filter_map(entry_kv)
            // Key 13: field formatting (data formats); key 7, the tab
            // order position, is the object's own.
            .filter(|(k, _)| *k == 13)
            .collect()
    };
    let mut renamed = Vec::new();
    for new in layouts {
        let Some(old) = old_all.iter().find(|l| l.id == new.id) else {
            continue;
        };
        if old == new {
            continue;
        }
        let l = new.id;
        let lp = [4, 5, l];
        let own_blob = object_blob(&entries, l);
        // Tab order [L].[21] as stored: key 128 + n → the n-th object.
        let tab_path = [4, 5, l, 21];
        let old_tabs: Vec<u64> = {
            let mut v: Vec<(u32, u64)> = entries
                .iter()
                .filter(|e| ipath(e).as_deref() == Some(&tab_path))
                .filter_map(entry_kv)
                .filter_map(|(k, d)| Some((k, index_ref(&d)? as u64)))
                .collect();
            v.sort_unstable();
            v.into_iter().map(|(_, id)| id).collect()
        };
        // Prototype: same kind and type, preferably on this layout.
        let proto_of = |o: &LayoutObject| -> Option<(u32, u64, Vec<u8>)> {
            let ty = crate::theme::ObjectStyle::parse(&o.style_raw).object_type;
            let same = old.objects.iter().find(|x| {
                x.kind == o.kind && crate::theme::ObjectStyle::parse(&x.style_raw).object_type == ty
            });
            if let Some(x) = same {
                let m = crate::pb::Msg::parse(&own_blob)?;
                let raw = m.msgs(2).find(|m| m.uint(2) == Some(x.id))?.encode();
                return Some((l, x.id, raw));
            }
            protos.get(&(o.kind, ty)).cloned()
        };
        let olds: BTreeMap<u64, &LayoutObject> = old.objects.iter().map(|o| (o.id, o)).collect();
        let news: BTreeMap<u64, &LayoutObject> = new.objects.iter().map(|o| (o.id, o)).collect();
        let deleted: BTreeSet<u64> = olds
            .keys()
            .filter(|i| !news.contains_key(i))
            .copied()
            .collect();
        let added: Vec<&LayoutObject> = new
            .objects
            .iter()
            .filter(|o| !olds.contains_key(&o.id))
            .collect();
        // Prototype message and node keys of each new object, read before
        // anything changes.
        let protos_of: BTreeMap<u64, (Vec<u8>, NodeKeys)> = added
            .iter()
            .filter_map(|o| {
                let (pl, pid, raw) = proto_of(o)?;
                Some((o.id, (raw, node_keys(&entries, pl, pid))))
            })
            .collect();
        let mut add: Vec<Entry> = Vec::new();

        // Object list.
        if old.objects != new.objects {
            let blob = &own_blob;
            let out = ly::encode_objects(blob, old, new, &|o| {
                protos_of.get(&o.id).map(|p| p.0.clone())
            });
            if out != *blob {
                entries.retain(|e| match (ipath(e).as_deref(), entry_kv(e)) {
                    (Some([4, 5, x]), Some((7, _))) => *x != l,
                    (Some([4, 5, x, 7, ..]), _) => *x != l,
                    _ => true,
                });
                if out.len() <= INLINE_MAX {
                    add.push(kv_entry(&lp, 7, out));
                } else {
                    let p = [4, 5, l, 7];
                    add.push(marker(&p));
                    add.push(kv_entry(&p, 0, (out.len() as u32).to_be_bytes().to_vec()));
                    for (i, c) in out.chunks(INLINE_MAX).enumerate() {
                        add.push(kv_entry(&p, i as u32 + 1, c.to_vec()));
                    }
                }
            }
        }

        // Per-object nodes: removed for deleted objects, rewritten keys for
        // edited ones, created for new ones.
        let node = |id: u64| [4, 5, l, 5, id as u32];
        entries.retain(|e| {
            let Some(p) = ipath(e) else { return true };
            if p.len() < 5 || p[..4] != [4, 5, l, 5] {
                return true;
            }
            let id = p[4] as u64;
            if deleted.contains(&id) {
                return false;
            }
            let (Some(o), Some(n)) = (olds.get(&id), news.get(&id)) else {
                return true;
            };
            let key = entry_kv(e).map(|x| x.0);
            match p.len() {
                5 => match key {
                    Some(1) => o.text == n.text,
                    Some(16) => o.name == n.name,
                    _ => true,
                },
                // Button label calculation and step parameters.
                _ if p[5] == 3 => o.label == n.label,
                // Tab label calculation of a panel.
                _ if p[5] == 2 && n.kind == 12 => o.text == n.text,
                _ if p[5] == 17 => {
                    o.button.as_ref().map(|s| s.id) == n.button.as_ref().map(|s| s.id)
                }
                _ => true,
            }
        });
        let calc_entries = |id: u64, sub: u32, tokens: &[u8]| -> Vec<Entry> {
            let p = [4, 5, l, 5, id as u32, sub];
            vec![
                marker(&p),
                kv_entry(&p, 4, vec![1, 0]),
                kv_entry(&p, 5, tokens.to_vec()),
                kv_entry(&p, 6, tokens.to_vec()),
            ]
        };
        for n in &new.objects {
            let Some(o) = olds.get(&n.id) else { continue };
            if o.text != n.text
                && let Some(t) = &n.text
            {
                add.push(kv_entry(&node(n.id), 1, encode_text(t)));
                if n.kind == 12 {
                    add.extend(calc_entries(n.id, 2, &text_literal_tokens(t)));
                }
            }
            if o.name != n.name {
                add.push(kv_entry(&node(n.id), 16, encode_text(&n.name)));
            }
            if o.label != n.label
                && let Some(t) = &n.label
            {
                add.extend(calc_entries(n.id, 3, t));
            }
        }
        for n in &added {
            add.push(marker(&node(n.id)));
            if let Some(t) = &n.text {
                add.push(kv_entry(&node(n.id), 1, encode_text(t)));
                if n.kind == 12 {
                    add.extend(calc_entries(n.id, 2, &text_literal_tokens(t)));
                }
            }
            for (k, v) in protos_of
                .get(&n.id)
                .map(|p| p.1.clone())
                .unwrap_or_default()
            {
                add.push(kv_entry(&node(n.id), k, v));
            }
            add.push(kv_entry(&node(n.id), 16, encode_text(&n.name)));
            let seed = (l as u64) << 32 | n.id;
            add.push(kv_entry(&node(n.id), 216, fresh_uuid(seed)));
            add.push(kv_entry(&node(n.id), 217, fresh_uuid(seed)));
            if let Some(t) = &n.label {
                add.extend(calc_entries(n.id, 3, t));
            }
        }
        // Last object id: `[4].[5].[L].[5]` key 0.
        let max_new = new.objects.iter().map(|o| o.id).max().unwrap_or(0);
        let mut last = 0;
        for e in &entries {
            if let (Some([4, 5, x, 5]), Some((0, d))) = (ipath(e).as_deref(), entry_kv(e))
                && *x == l
            {
                last = read_len_int(&d).unwrap_or(0);
            }
        }
        if max_new > last {
            entries.retain(|e| {
                !(ipath(e).as_deref() == Some(&[4, 5, l, 5])
                    && entry_kv(e).is_some_and(|x| x.0 == 0))
            });
            if !entries
                .iter()
                .any(|e| ipath(e).as_deref() == Some(&[4, 5, l, 5]) && e.item == [0x80])
            {
                add.push(marker(&[4, 5, l, 5]));
            }
            add.push(kv_entry(&[4, 5, l, 5], 0, len_int(max_new)));
        }

        // Spatial index [L].[10]: key (top u16, left u16, id − 1 u16) of the
        // object's absolute position; tab order [L].[21].
        let mut index_add = Vec::new();
        entries.retain(|e| {
            let p = ipath(e);
            let in_tabs = p.as_deref() == Some(&[4, 5, l, 21]);
            let in_index = p.as_deref() == Some(&[4, 5, l, 10]);
            if !in_tabs && !in_index || e.item.first() == Some(&0x80) {
                return true;
            }
            let Ok(chunks) = crate::chunk::parse(&e.item) else {
                return true;
            };
            let (key, data) = match chunks.first() {
                Some(crate::Chunk::KvLong { key, data }) => (Some(key.to_vec()), data.to_vec()),
                Some(crate::Chunk::Kv { data, .. }) => (None, data.to_vec()),
                _ => return true,
            };
            let Some(id) = index_ref(&data).map(|x| x as u64) else {
                return true;
            };
            if deleted.contains(&id) {
                return false;
            }
            if let (Some(k), Some(o), Some(n)) = (key, olds.get(&id), news.get(&id))
                && k.len() == 6
            {
                let (a, b) = (old.absolute(o), new.absolute(n));
                if (a.top, a.left) != (b.top, b.left) {
                    let at = |i: usize| u16::from_be_bytes([k[i], k[i + 1]]) as f64;
                    let mv =
                        |v: f64, d: f64| ((v + d).round().clamp(0.0, 65535.0) as u16).to_be_bytes();
                    let mut nk = mv(at(0), b.top - a.top).to_vec();
                    nk.extend(mv(at(2), b.left - a.left));
                    nk.extend_from_slice(&k[4..]);
                    index_add.push(Entry {
                        path: e.path.clone(),
                        item: write::encode(&Item::KvLong { key: nk, data }),
                    });
                    return false;
                }
            }
            true
        });
        add.extend(index_add);
        for n in added
            .iter()
            .filter(|n| n.parent.is_none() && indexed_kind(n.kind))
        {
            let b = new.absolute(n);
            let u = |v: f64| (v.round().clamp(0.0, 65535.0) as u16).to_be_bytes();
            let mut key = u(b.top).to_vec();
            key.extend(u(b.left));
            key.extend(((n.id.saturating_sub(1)) as u16).to_be_bytes());
            add.push(Entry {
                path: spelled(&[4, 5, l, 10]),
                item: write::encode(&Item::KvLong {
                    key,
                    data: object_ref(n.id),
                }),
            });
        }

        // Tab order: the object's node key 7 = `00 n`. New fields go to the end.
        let mut tabs: Vec<u64> = old_tabs
            .iter()
            .copied()
            .filter(|id| news.contains_key(id))
            .collect();
        tabs.extend(added.iter().filter(|n| n.kind == 1).map(|n| n.id));
        if tabs != old_tabs {
            entries.retain(|e| {
                let p = ipath(e);
                let in_tabs = p.as_deref() == Some(&tab_path) && e.item != [0x80];
                let tab_key = p.as_ref().is_some_and(|p| {
                    p.len() == 5 && p[..4] == [4, 5, l, 5] && old_tabs.contains(&(p[4] as u64))
                }) && entry_kv(e).is_some_and(|x| x.0 == 7);
                !in_tabs && !tab_key
            });
            if !entries
                .iter()
                .any(|e| ipath(e).as_deref() == Some(&tab_path) && e.item == [0x80])
            {
                add.push(marker(&tab_path));
            }
            for (i, id) in tabs.iter().enumerate() {
                add.push(kv_entry(&tab_path, 129 + i as u32, object_ref(*id)));
                add.push(kv_entry(
                    &node(*id),
                    7,
                    (i as u16 + 1).to_be_bytes().to_vec(),
                ));
            }
        }

        // Parts: patched in place, or the whole `[L].[3]` rewritten when
        // parts were added, removed or reordered. Each part lists the
        // top-level objects whose top lies in it (key 7).
        let lists: Vec<Vec<u8>> = new
            .parts
            .iter()
            .map(|p| {
                let mut ids: Vec<u64> = new
                    .objects
                    .iter()
                    .filter(|o| o.parent.is_none())
                    .filter(|o| o.bounds.top >= p.top && o.bounds.top < p.top + p.height)
                    .map(|o| o.id)
                    .collect();
                ids.sort_unstable();
                let mut v = (ids.len() as u32).to_be_bytes().to_vec();
                for id in ids {
                    let r = [vec![1], path_elem(id as u32)].concat();
                    v.push(r.len() as u8);
                    v.extend(r);
                }
                v
            })
            .collect();
        let same_parts = old.parts.len() == new.parts.len()
            && old
                .parts
                .iter()
                .zip(&new.parts)
                .all(|(a, b)| (a.id, a.kind) == (b.id, b.kind));
        let raw_parts: BTreeMap<u32, Vec<u8>> = entries
            .iter()
            .filter_map(|e| match (ipath(e).as_deref(), entry_kv(e)) {
                (Some([4, 5, x, 3, p]), Some((2, d))) if *x == l => Some((*p, d)),
                _ => None,
            })
            .collect();
        let part_msg = |n: &crate::layouts::Part| -> Vec<u8> {
            match (
                raw_parts.get(&n.id),
                old.parts
                    .iter()
                    .find(|o| (o.id, o.kind) == (n.id, n.kind) && n.id != 0),
            ) {
                (Some(raw), Some(o)) => ly::patch_part(raw, o, n),
                _ => ly::new_part(n),
            }
        };
        if same_parts {
            for e in entries.iter_mut() {
                let ip = ipath(e);
                let (Some([4, 5, x, 3, p]), Some((k, d))) = (ip.as_deref(), entry_kv(e)) else {
                    continue;
                };
                let Some(i) = new.parts.iter().position(|n| n.id == *p) else {
                    continue;
                };
                if *x != l {
                    continue;
                }
                let out = match k {
                    2 => part_msg(&new.parts[i]),
                    7 => lists[i].clone(),
                    _ => continue,
                };
                if out != d {
                    e.item = write::encode(&Item::Kv { key: k, data: out });
                }
            }
        } else {
            entries.retain(|e| {
                !matches!(ipath(e).as_deref(), Some([4, 5, x, 3, ..]) if *x == l)
                    || e.path.len() == 4 && e.item == [0x80]
            });
            let pp = [4, 5, l, 3];
            add.push(kv_entry(&pp, 0, len_int(new.parts.len() as u64)));
            for (i, n) in new.parts.iter().enumerate() {
                let p = [4, 5, l, 3, i as u32 + 1];
                add.push(marker(&p));
                add.push(kv_entry(&p, 2, part_msg(n)));
                add.push(kv_entry(&p, 7, lists[i].clone()));
            }
        }

        // Header: table occurrence and width; name.
        for e in entries.iter_mut() {
            let p = ipath(e);
            let Some((key, d)) = entry_kv(e) else {
                continue;
            };
            let header = p.as_deref() == Some(&lp) && key == 2;
            let listed = p.as_deref() == Some(&[4, 1, 7, l]) && key == 2;
            if (header || listed)
                && (old.table_occurrence != new.table_occurrence
                    || header && old.width != new.width)
                && let Some(mut m) = crate::pb::MsgBuf::parse(&d)
            {
                if old.table_occurrence != new.table_occurrence {
                    m.set(2, crate::pb::Field::Varint(new.table_occurrence));
                }
                if header && old.width != new.width {
                    m.set(8, crate::pb::Field::F64(new.width));
                }
                e.item = write::encode(&Item::Kv {
                    key: 2,
                    data: m.encode(),
                });
            }
        }
        if old.name != new.name {
            renamed.push((l, new.name.clone()));
        }
        entries.extend(add);
    }
    write::sort(&mut entries);
    let out = write::build(&file.bytes()[..crate::SECTOR], &entries);
    if renamed.is_empty() {
        return Ok(out);
    }
    // Names: both name keys and the collation-keyed name index.
    let mut ed = crate::schema_edit::Editor::open(&File::from_bytes(out)?)?;
    for (l, name) in renamed {
        // An empty name is refused by the editor and kept as it was.
        let _ = ed.rename_layout(l, &name);
    }
    Ok(ed.finish())
}

/// Reference to a layout object in its layout's indexes: `00 len 01 id`,
/// then `00 00 00 00 01` (spatial index `[L].[10]`, tab order `[L].[21]`).
fn object_ref(id: u64) -> Vec<u8> {
    let r = [vec![1], path_elem(id as u32)].concat();
    let mut data = vec![0, r.len() as u8];
    data.extend(r);
    data.extend([0, 0, 0, 0, 1]);
    data
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
