//! Writer: re-serialises a logical key/value store into an HBAM7 file.
//!
//! The store is a list of [`Entry`] in leaf order. [`build`] packs it into
//! 4096-byte leaf sectors (each restarting its path), then builds index
//! levels above them until a single root remains at sector 1.

use crate::{Chunk, Error, File, SECTOR};

const HEADER_LEN: usize = 20;
const PAYLOAD: usize = SECTOR - HEADER_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Data(Vec<u8>),
    Kv { key: u32, data: Vec<u8> },
    KvLong { key: Vec<u8>, data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<Vec<u8>>,
    pub item: Item,
}

/// The logical content of a file, in leaf order.
pub fn entries(file: &File) -> Result<Vec<Entry>, Error> {
    let mut out = Vec::new();
    file.walk(|path, c| {
        let item = match c {
            Chunk::Data(d) => Item::Data(d.to_vec()),
            Chunk::Kv { key, data } => Item::Kv {
                key: *key,
                data: data.to_vec(),
            },
            Chunk::KvLong { key, data } => Item::KvLong {
                key: key.to_vec(),
                data: data.to_vec(),
            },
            Chunk::Push(_) | Chunk::Pop => return,
        };
        out.push(Entry {
            path: path.iter().map(|p| p.to_vec()).collect(),
            item,
        });
    })?;
    Ok(out)
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

/// Path-integer encoding of a key (1 byte below 128, else 2 bytes).
fn key_bytes(k: u32) -> Vec<u8> {
    if k < 0x80 {
        vec![k as u8]
    } else {
        let v = k - 0x80;
        vec![0x80 | (v >> 8) as u8, v as u8]
    }
}

fn encode_item(item: &Item, out: &mut Vec<u8>) {
    match item {
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
            let short = *key < 0x80;
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
            match (key.len(), n <= 255) {
                (3, true) => out.push(0x16),
                (3, false) => out.push(0x17),
                (k, true) => out.extend_from_slice(&[0x1E, k as u8]),
                (k, false) => out.extend_from_slice(&[0x1F, k as u8]),
            }
            out.extend_from_slice(key);
            if n <= 255 {
                out.push(n as u8);
            } else {
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            out.extend_from_slice(data);
        }
    }
}

/// Bytes that move from path `from` to `to`.
fn path_delta(from: &[Vec<u8>], to: &[Vec<u8>], out: &mut Vec<u8>) {
    let common = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    out.extend(std::iter::repeat_n(0x40, from.len() - common));
    for e in &to[common..] {
        push_code(e, out);
    }
}

struct Page {
    payload: Vec<u8>,
    /// Path and key of the first entry, for the parent index.
    first: Entry,
}

/// Packs entries into pages of at most PAYLOAD bytes.
fn pack(entries: &[Entry]) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut cur: Option<(Vec<u8>, Vec<Vec<u8>>, Entry)> = None;
    for e in entries {
        let mut item = Vec::new();
        encode_item(&e.item, &mut item);
        if let Some((buf, path, _)) = &mut cur {
            let mut d = Vec::new();
            path_delta(path, &e.path, &mut d);
            if buf.len() + d.len() + item.len() <= PAYLOAD {
                buf.extend_from_slice(&d);
                buf.extend_from_slice(&item);
                *path = e.path.clone();
                continue;
            }
            let (payload, _, first) = cur.take().unwrap();
            pages.push(Page { payload, first });
        }
        let mut buf = Vec::new();
        path_delta(&[], &e.path, &mut buf);
        buf.extend_from_slice(&item);
        cur = Some((buf, e.path.clone(), e.clone()));
    }
    if let Some((payload, _, first)) = cur {
        pages.push(Page { payload, first });
    }
    pages
}

fn index_entry(first: &Entry, child: u32) -> Entry {
    let key = match &first.item {
        Item::Kv { key, .. } => *key,
        _ => 0,
    };
    Entry {
        path: first.path.clone(),
        item: Item::Kv {
            key,
            data: child.to_be_bytes().to_vec(),
        },
    }
}

/// Builds a complete file. `header` is the 4096-byte header sector to keep.
pub fn build(header: &[u8], entries: &[Entry]) -> Vec<u8> {
    // Levels bottom-up; ids are assigned once the root is known (root = 1).
    let mut levels: Vec<Vec<Page>> = vec![pack(entries)];
    while levels.last().unwrap().len() > 1 {
        // Child ids are placeholders here and fixed up below.
        let idx: Vec<Entry> = levels
            .last()
            .unwrap()
            .iter()
            .map(|p| index_entry(&p.first, 0))
            .collect();
        levels.push(pack(&idx));
    }
    // A single leaf still needs a root index sector.
    if levels.len() == 1 {
        let idx = vec![index_entry(&levels[0][0].first, 0)];
        levels.push(pack(&idx));
    }
    // Assign ids: root = 1, then level by level downwards.
    let mut ids: Vec<Vec<u32>> = vec![Vec::new(); levels.len()];
    let mut next = 1;
    for (l, pages) in levels.iter().enumerate().rev() {
        ids[l] = (next..next + pages.len() as u32).collect();
        next += pages.len() as u32;
    }
    // Re-pack index levels with real child ids.
    for l in 1..levels.len() {
        let idx: Vec<Entry> = levels[l - 1]
            .iter()
            .zip(&ids[l - 1])
            .map(|(p, id)| index_entry(&p.first, *id))
            .collect();
        let repacked = pack(&idx);
        assert_eq!(
            repacked.len(),
            levels[l].len(),
            "index re-pack changed size"
        );
        levels[l] = repacked;
    }
    let total = next - 1;
    let mut out = header.to_vec();
    out.resize(SECTOR * (total as usize + 1), 0);
    for (l, pages) in levels.iter().enumerate() {
        for (i, p) in pages.iter().enumerate() {
            let id = ids[l][i];
            let prev = if i == 0 { 0 } else { ids[l][i - 1] };
            // Sector 1's "next" holds the sector count.
            let nxt = if id == 1 {
                total
            } else {
                ids[l].get(i + 1).copied().unwrap_or(0)
            };
            let s = &mut out[id as usize * SECTOR..(id as usize + 1) * SECTOR];
            s[4..8].copy_from_slice(&prev.to_be_bytes());
            s[8..12].copy_from_slice(&nxt.to_be_bytes());
            s[12..14].copy_from_slice(&(l as u16).to_be_bytes());
            s[14..16].copy_from_slice(&((PAYLOAD - p.payload.len()) as u16).to_be_bytes());
            s[16..20].copy_from_slice(&1u32.to_be_bytes());
            s[HEADER_LEN..HEADER_LEN + p.payload.len()].copy_from_slice(&p.payload);
        }
    }
    out
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
            let mut b = Vec::new();
            encode_item(it, &mut b);
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
