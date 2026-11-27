//! Writer: re-serialises a logical key/value store into an HBAM7 file.
//!
//! The store is a list of [`Entry`] in leaf order. [`build`] packs it into
//! 4096-byte leaf sectors (each restarting its path), then builds index
//! levels above them until a single root remains at sector 1.

use crate::{Chunk, Error, File, SECTOR};

const HEADER_LEN: usize = 20;
const PAYLOAD: usize = SECTOR - HEADER_LEN;

/// A value to encode with [`encode`], for content built from scratch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Data(Vec<u8>),
    Kv {
        key: u32,
        data: Vec<u8>,
    },
    KvLong {
        key: Vec<u8>,
        data: Vec<u8>,
    },
    /// [`Chunk::Marker`].
    Marker,
}

/// One chunk of the store at its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<Vec<u8>>,
    /// The encoded chunk: opcode (no pop flag) and operands. Entries read
    /// from a file keep FileMaker's bytes: the same key can be spelled
    /// several ways (`06 D8` and `0E 80 58` both read as key 216) and the
    /// spelling decides where it sorts.
    pub item: Vec<u8>,
}

/// The logical content of a file, in leaf order. A node pushed and popped
/// with nothing inside becomes an entry with an empty item.
pub fn entries(file: &File) -> Result<Vec<Entry>, Error> {
    let mut out = Vec::new();
    for s in file.leaves() {
        out.extend(leaf_entries(s.payload).map_err(|e| Error::Sector {
            id: s.id,
            source: Box::new(e),
        })?);
    }
    Ok(out)
}

/// The entries of one leaf payload.
pub fn leaf_entries(payload: &[u8]) -> Result<Vec<Entry>, Error> {
    let mut out = Vec::new();
    let mut path: Vec<&[u8]> = Vec::new();
    let mut pushed = false;
    for (c, raw) in crate::chunk::parse_spans(payload)? {
        match c {
            Chunk::Push(p) => {
                path.push(p);
                pushed = true;
                continue;
            }
            Chunk::Pop => {
                if pushed {
                    out.push(entry(&path, &[]));
                }
                path.pop();
            }
            _ => out.push(entry(&path, raw)),
        }
        pushed = false;
    }
    Ok(out)
}

/// Entry for an item chunk decoded from `raw` (as in [`crate::chunk::parse_spans`]).
fn entry(path: &[&[u8]], raw: &[u8]) -> Entry {
    let mut item = raw.to_vec();
    if let Some(op) = item.first_mut().filter(|op| **op & 0xC0 == 0xC0) {
        *op &= 0x3F;
    }
    Entry {
        path: path.iter().map(|p| p.to_vec()).collect(),
        item,
    }
}

/// Encodes a new item.
pub fn encode(item: &Item) -> Vec<u8> {
    let mut b = Vec::new();
    encode_item(item, &mut b);
    b
}

fn push_code(e: &[u8], out: &mut Vec<u8>) {
    match e.len() {
        1 => out.push(0x20),
        2 => out.push(0x28),
        3 => out.push(0x30),
        8 => out.extend_from_slice(&[0x20, 0xFE]),
        n => out.extend_from_slice(&[0x38, n as u8]),
    }
    out.extend_from_slice(e);
}

/// Key bytes: a raw byte below 256 (ops 01–07), else a 2-byte path
/// integer (ops 09–0F). FileMaker never uses the 2-byte form below 256.
fn key_bytes(k: u32) -> Vec<u8> {
    if k < 0x100 {
        vec![k as u8]
    } else {
        let v = k - 0x80;
        vec![0x80 | (v >> 8) as u8, v as u8]
    }
}

fn encode_item(item: &Item, out: &mut Vec<u8>) {
    match item {
        Item::Marker => out.push(0x80),
        Item::Data(d) => {
            let n = d.len();
            match n {
                1 if d[0] != 0 => out.push(0x00),
                2 => out.push(0x08),
                3 => out.push(0x10),
                4 => out.push(0x11),
                5 | 7 | 9 | 11 => out.push(0x11 + ((n - 3) / 2) as u8),
                _ => out.extend_from_slice(&[0x18, n as u8]),
            }
            out.extend_from_slice(d);
        }
        Item::Kv { key, data } => {
            let n = data.len();
            let short = *key < 0x100;
            let kb = key_bytes(*key);
            // 0x0E followed by 0xFF reads as a 6-byte data chunk: avoid it.
            let ff = !short && kb[0] == 0xFF;
            let base = if short { 0x01 } else { 0x09 };
            match n {
                1 => out.push(base),
                2 | 4 | 6 | 8 => out.push(base + (n / 2) as u8),
                _ if n <= 255 && !ff => out.push(base + 5),
                _ => out.push(base + 6),
            }
            out.extend_from_slice(&kb);
            match n {
                1 | 2 | 4 | 6 | 8 => {}
                _ if n <= 255 && !ff => out.push(n as u8),
                _ => out.extend_from_slice(&(n as u16).to_be_bytes()),
            }
            out.extend_from_slice(data);
        }
        Item::KvLong { key, data } => {
            let n = data.len();
            let k = key.len();
            match (k, n) {
                // 0x1B 00 is reserved for a short-key form, so no empty key there.
                (_, 1) => out.extend_from_slice(&[0x19, k as u8]),
                (_, 2 | 6 | 8) | (1.., 4) => {
                    out.extend_from_slice(&[0x19 + (n / 2) as u8, k as u8])
                }
                (3, _) if n <= 255 => out.push(0x16),
                (3, _) => out.push(0x17),
                _ if n <= 255 => out.extend_from_slice(&[0x1E, k as u8]),
                _ => out.extend_from_slice(&[0x1F, k as u8]),
            }
            out.extend_from_slice(key);
            match (k, n) {
                (_, 1 | 2 | 6 | 8) | (1.., 4) => {}
                _ if n <= 255 => out.push(n as u8),
                _ => out.extend_from_slice(&(n as u16).to_be_bytes()),
            }
            out.extend_from_slice(data);
        }
    }
}

/// Bytes that move from path `from` to `to`: pops, then pushes. Returns
/// the number of pops.
fn path_delta(from: &[Vec<u8>], to: &[Vec<u8>], out: &mut Vec<u8>) -> usize {
    let common = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    let pops = from.len() - common;
    out.extend(std::iter::repeat_n(0x40, pops));
    for e in &to[common..] {
        push_code(e, out);
    }
    pops
}

/// Packs entries into leaf payloads of at most PAYLOAD bytes, each with the
/// first entry it holds.
pub fn pack(entries: &[Entry]) -> Vec<(Vec<u8>, &Entry)> {
    let mut pages: Vec<(Vec<u8>, &Entry)> = Vec::new();
    let mut prev: &[Vec<u8>] = &[];
    for e in entries {
        let mut d = Vec::new();
        let pops = path_delta(prev, &e.path, &mut d);
        let fits = pages
            .last()
            .is_some_and(|(buf, _)| buf.len() + d.len() + e.item.len() <= PAYLOAD);
        if !fits {
            // Like FileMaker, keep the pops towards the next entry.
            if let Some((buf, _)) = pages.last_mut()
                && buf.len() + pops <= PAYLOAD
            {
                buf.extend_from_slice(&d[..pops]);
            }
            // Each sector restarts from an empty path.
            d.clear();
            path_delta(&[], &e.path, &mut d);
            pages.push((Vec::new(), e));
        }
        // Always a page here: one was pushed above unless the entry fit.
        if let Some((buf, _)) = pages.last_mut() {
            buf.extend_from_slice(&d);
            buf.extend_from_slice(&e.item);
        }
        prev = &e.path;
    }
    pages
}

/// One index entry: a separator (path, then key bytes) and the child sector.
///
/// `key` is the item opcode without the pop flag, followed by the key and
/// without the 4-byte child id: `03 k`, `0B k k`, `13 k k k` (Kv with a 1,
/// 2 or 3-byte key), `1B len key` (long key), or [`LOW`] / `1B 00 FF`
/// (the path alone, sorting before / after everything under it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sep {
    pub path: Vec<Vec<u8>>,
    pub key: Vec<u8>,
    pub child: u32,
}

/// Separator "the path itself", sorting before everything under it.
pub const LOW: [u8; 3] = [0x1B, 0x00, 0x01];

/// Key bytes of the separator naming `e` exactly, read off its opcode.
/// Ops 00–07, 08–0F and 10–15 are Kv with a 1, 2 or 3-byte key (00, 08
/// and 10 with an empty value); a [`Chunk::Marker`] sorts as the bare path.
/// ponytail: ops 18/23 and `1B 00` (never seen in the templates) also get
/// [`LOW`], which is only valid when the left neighbour lies outside the
/// path; work out their sort key if real files use them.
fn sep_key(e: &Entry) -> Vec<u8> {
    let b = &e.item;
    match b.first().copied().unwrap_or(0x80) {
        0x00..=0x07 => vec![0x03, b[1]],
        0x08..=0x0F => vec![0x0B, b[1], b[2]],
        0x10..=0x15 => vec![0x13, b[1], b[2], b[3]],
        0x16 | 0x17 => [&[0x1B, 3], &b[1..4]].concat(),
        0x19..=0x1F if b[1] > 0 => [&[0x1B], &b[1..2 + b[1] as usize]].concat(),
        _ => LOW.to_vec(),
    }
}

/// Key bytes an item sorts by, as written (no opcode); empty for a node
/// marker or an empty node.
fn item_key(b: &[u8]) -> &[u8] {
    match b.first() {
        Some(0x1B) if b.get(1) == Some(&0) => &b[2..3],
        Some(0x00..=0x07) => &b[1..2],
        Some(0x08..=0x0F) => &b[1..3],
        Some(0x10..=0x17) => &b[1..4],
        Some(0x19..=0x1F) => &b[2..2 + b[1] as usize],
        _ => &[],
    }
}

/// FileMaker's store order: path elements, then the item's key, compared
/// as byte strings element by element (a prefix sorts first).
pub fn sort_key(e: &Entry) -> Vec<&[u8]> {
    let mut k: Vec<&[u8]> = e.path.iter().map(|p| p.as_slice()).collect();
    let key = item_key(&e.item);
    if !e.item.is_empty() && e.item[0] != 0x80 {
        k.push(key);
    }
    k
}

/// Sorts entries into FileMaker's store order (stable).
pub fn sort(entries: &mut Vec<Entry>) {
    drop_leftovers(entries);
    entries.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
}

/// Removes empty nodes whose path also holds content: FileMaker leaves them
/// at sector boundaries (a push popped at once), out of store order. Other
/// empty nodes are content (e.g. `[TO].[7].[1].[…]` cache keys) and stay.
pub fn drop_leftovers(entries: &mut Vec<Entry>) {
    let full: std::collections::HashSet<Vec<Vec<u8>>> = entries
        .iter()
        .filter(|e| !e.item.is_empty())
        .map(|e| e.path.clone())
        .collect();
    entries.retain(|e| !e.item.is_empty() || !full.contains(&e.path));
}

fn sep(e: &Entry, child: u32) -> Sep {
    Sep {
        path: e.path.clone(),
        key: sep_key(e),
        child,
    }
}

/// Encodes one index sector. A run of pushes builds the separator's path
/// on a stack; the entry's item ends it. A pop flag (`| 0xC0`) on the item
/// pops, after it, every flagged element on top of the stack and then one
/// unflagged element. So the flag is set on every element that leaves the
/// stack together with the one below it, and on all of them for the last
/// entry (the sector ends with an empty stack).
pub fn encode_index(seps: &[Sep]) -> Vec<u8> {
    let n = seps.len();
    // Stack depth kept after entry i (None for the last entry: pop all).
    let keep: Vec<Option<usize>> = (0..n)
        .map(|i| {
            seps.get(i + 1).map(|next| {
                let p = &seps[i].path;
                p.iter().zip(&next.path).take_while(|(a, b)| a == b).count()
            })
        })
        .collect();
    let mut out = Vec::new();
    let mut depth = 0;
    for (i, s) in seps.iter().enumerate() {
        for (d, e) in s.path.iter().enumerate().skip(depth) {
            // The element at depth d is popped by the first entry keeping < d + 1.
            // keep[n - 1] is None, so the search always stops by then.
            let j = (i..n)
                .find(|&j| keep[j].is_none_or(|k| k <= d))
                .unwrap_or(n - 1);
            let start = out.len();
            push_code(e, &mut out);
            if keep[j] != Some(d) {
                out[start] |= 0xC0;
            }
        }
        let pop = keep[i].is_none_or(|k| k < s.path.len());
        out.push(s.key[0] | if pop { 0xC0 } else { 0 });
        out.extend_from_slice(&s.key[1..]);
        out.extend_from_slice(&s.child.to_be_bytes());
        depth = keep[i].map_or(0, |k| k.min(s.path.len()));
    }
    out
}

/// Decodes an index sector payload (inverse of [`encode_index`]).
pub fn decode_index(payload: &[u8]) -> Result<Vec<Sep>, Error> {
    let mut stack: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut out = Vec::new();
    let mut i = 0;
    let take = |i: &mut usize, n: usize| -> Result<&[u8], Error> {
        let s = payload.get(*i..*i + n).ok_or(Error::Truncated(*i))?;
        *i += n;
        Ok(s)
    };
    while i < payload.len() && payload[i] != 0 {
        let at = i;
        let raw = payload[i];
        i += 1;
        let flag = raw & 0xC0 == 0xC0;
        let op = if flag { raw & 0x3F } else { raw };
        let elem = match op {
            0x20 if payload.get(i) == Some(&0xFE) => {
                i += 1;
                take(&mut i, 8)?
            }
            0x20 => take(&mut i, 1)?,
            0x28 => take(&mut i, 2)?,
            0x30 => take(&mut i, 3)?,
            0x38 => {
                let n = take(&mut i, 1)?[0] as usize;
                take(&mut i, n)?
            }
            0x03 | 0x0B | 0x13 | 0x1B => {
                let klen = match op {
                    0x03 => 1,
                    0x0B => 2,
                    0x13 => 3,
                    _ => match take(&mut i, 1)?[0] {
                        0 => 1,
                        n => n as usize,
                    },
                };
                i += klen;
                let mut key = payload.get(at..i).ok_or(Error::Truncated(at))?.to_vec();
                key[0] = op;
                let c = take(&mut i, 4)?;
                out.push(Sep {
                    path: stack.iter().map(|(e, _)| e.clone()).collect(),
                    key,
                    child: u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                });
                if flag {
                    while stack.last().is_some_and(|e| e.1) {
                        stack.pop();
                    }
                    stack.pop();
                }
                continue;
            }
            _ => return Err(Error::UnknownOpcode { op: raw, at }),
        };
        stack.push((elem.to_vec(), flag));
    }
    Ok(out)
}

/// Splits separators into index sectors of at most PAYLOAD bytes.
fn pack_index(seps: &[Sep]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    // ponytail: re-encodes the open sector per entry (quadratic in entries
    // per sector, ~500 at most); track the size incrementally if it shows.
    for end in 1..=seps.len() {
        if end - start > 1 && encode_index(&seps[start..end]).len() > PAYLOAD {
            out.push(start..end - 1);
            start = end - 1;
        }
    }
    if start < seps.len() {
        out.push(start..seps.len());
    }
    out
}

/// Builds a complete file. `header` is the 4096-byte header sector to keep.
pub fn build(header: &[u8], entries: &[Entry]) -> Vec<u8> {
    let leaves = pack(entries);
    // Separator of each node at the current level: its first key.
    let mut firsts: Vec<Sep> = leaves.iter().map(|(_, e)| sep(e, 0)).collect();
    // Index levels bottom-up, as ranges over the level below.
    let mut levels: Vec<Vec<std::ops::Range<usize>>> = Vec::new();
    loop {
        let ranges = pack_index(&firsts);
        firsts = ranges.iter().map(|r| firsts[r.start].clone()).collect();
        levels.push(ranges);
        if firsts.len() == 1 {
            break;
        }
    }
    // Ids: root = 1, then level by level downwards, leaves last.
    let counts: Vec<usize> = std::iter::once(leaves.len())
        .chain(levels.iter().map(|l| l.len()))
        .collect();
    let mut base = vec![0u32; counts.len()];
    let mut next = 1;
    for l in (0..counts.len()).rev() {
        base[l] = next;
        next += counts[l] as u32;
    }
    let total = next - 1;
    let mut out = header.to_vec();
    out.resize(SECTOR * (total as usize + 1), 0);
    let mut write = |level: usize, i: usize, payload: &[u8]| {
        let id = base[level] + i as u32;
        let prev = if i == 0 { 0 } else { id - 1 };
        // Sector 1's "next" holds the sector count.
        let nxt = match id {
            1 => total,
            _ if i + 1 < counts[level] => id + 1,
            _ => 0,
        };
        let s = &mut out[id as usize * SECTOR..(id as usize + 1) * SECTOR];
        s[4..8].copy_from_slice(&prev.to_be_bytes());
        s[8..12].copy_from_slice(&nxt.to_be_bytes());
        s[12..14].copy_from_slice(&(level as u16).to_be_bytes());
        s[14..16].copy_from_slice(&((PAYLOAD - payload.len()) as u16).to_be_bytes());
        s[16..20].copy_from_slice(&1u32.to_be_bytes());
        s[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    };
    for (i, (payload, _)) in leaves.iter().enumerate() {
        write(0, i, payload);
    }
    let mut seps: Vec<Sep> = leaves
        .iter()
        .enumerate()
        .map(|(i, (_, e))| sep(e, base[0] + i as u32))
        .collect();
    for (l, ranges) in levels.iter().enumerate() {
        let mut up = Vec::new();
        for (i, r) in ranges.iter().enumerate() {
            let mut part = seps[r.clone()].to_vec();
            up.push(Sep {
                child: base[l + 1] + i as u32,
                ..part[0].clone()
            });
            // FileMaker starts the leftmost sector of a level with the
            // lowest possible separator.
            if i == 0 {
                part[0].path.clear();
                part[0].key = LOW.to_vec();
            }
            write(l + 1, i, &encode_index(&part));
        }
        seps = up;
    }
    out
}

/// Experiment helper: payload of an index sector for the given leaves of
/// `file`, built the way [`build`] does.
pub fn index_payload(file: &File, leaves: &[u32]) -> Result<Vec<u8>, Error> {
    let mut seps = Vec::new();
    for id in leaves {
        let es = leaf_entries(file.sector(*id).payload)?;
        seps.push(sep(&es[0], *id));
    }
    if let Some(s) = seps.first_mut() {
        s.path.clear();
        s.key = LOW.to_vec();
    }
    Ok(encode_index(&seps))
}

/// Rewrites `file` from its logical content.
pub fn rewrite(file: &File) -> Result<Vec<u8>, Error> {
    Ok(build(&file.bytes()[..SECTOR], &entries(file)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_roundtrip_through_parser() {
        let items = [
            Item::Data(vec![7]),
            Item::Data(vec![0]),
            Item::Data(vec![1, 2, 3, 4, 5, 6, 7]),
            Item::Data(vec![9; 40]),
            Item::Kv {
                key: 16,
                data: vec![1],
            },
            Item::Kv {
                key: 3,
                data: vec![1, 2, 3, 4],
            },
            Item::Kv {
                key: 300,
                data: vec![5; 3],
            },
            Item::Kv {
                key: 32640,
                data: vec![5; 3],
            },
            Item::Kv {
                key: 2,
                data: vec![1; 900],
            },
            Item::KvLong {
                key: vec![1, 2, 3],
                data: vec![4; 10],
            },
            Item::KvLong {
                key: vec![1, 2],
                data: vec![4; 300],
            },
        ];
        for it in &items {
            let b = encode(it);
            let parsed = crate::chunk::parse(&b).unwrap();
            let back = match parsed[0] {
                Chunk::Data(d) => Item::Data(d.to_vec()),
                Chunk::Kv { key, data } => Item::Kv {
                    key,
                    data: data.to_vec(),
                },
                Chunk::KvLong { key, data } => Item::KvLong {
                    key: key.to_vec(),
                    data: data.to_vec(),
                },
                _ => panic!(),
            };
            assert_eq!(&back, it, "{b:02x?}");
        }
    }
}
