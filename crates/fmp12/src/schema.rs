//! Decoding of tables, fields and records from the path store.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Internal id (path element, >= 128).
    pub id: u32,
    pub name: String,
    /// Field id → name.
    pub fields: BTreeMap<u32, String>,
}

/// Record id → (field id → value).
pub type Records = BTreeMap<u32, BTreeMap<u32, String>>;

/// Accumulates a value stored either inline (key 16 of `node`) or as a long
/// value in sub-path `[node..., 16]` split over keys/segments 1, 2, ….
fn name_part(rel: &[u32], c: &Chunk) -> Option<(bool, String)> {
    match (rel, c) {
        ([], Chunk::Kv { key: 16, data }) => Some((true, text(data))),
        ([16], Chunk::Kv { key, data }) if *key >= 1 => Some((*key == 1, text(data))),
        ([16], Chunk::Segment { index, data }) if *index >= 1 => Some((*index == 1, text(data))),
        _ => None,
    }
}

fn add(map: &mut BTreeMap<u32, String>, id: u32, (first, s): (bool, String)) {
    let e = map.entry(id).or_default();
    if first {
        e.clear();
    }
    e.push_str(&s);
}

pub fn tables(file: &File) -> Result<Vec<Table>, Error> {
    let mut names = BTreeMap::new();
    let mut fields: BTreeMap<u32, BTreeMap<u32, String>> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match p.as_slice() {
            [3, 16, 5, t, rel @ ..] if *t >= 128 => {
                if let Some(part) = name_part(rel, c) {
                    add(&mut names, *t, part);
                }
            }
            [t, 3, 5, f, rel @ ..] if *t >= 128 => {
                if let Some(part) = name_part(rel, c) {
                    add(fields.entry(*t).or_default(), *f, part);
                }
            }
            _ => {}
        }
    })?;
    Ok(names
        .into_iter()
        .map(|(id, name)| Table {
            id,
            name,
            fields: fields.remove(&id).unwrap_or_default(),
        })
        .collect())
}

pub fn records(file: &File, table: &Table) -> Result<Records, Error> {
    let t = table.id;
    let mut out = Records::new();
    // ponytail: repeating-field repetitions (2-byte keys) not split yet.
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match (p.as_slice(), c) {
            ([pt, 5, r], Chunk::Kv { key, data }) if *pt == t => {
                out.entry(*r).or_default().insert(*key, text(data));
            }
            ([pt, 5, r, f], Chunk::Kv { key, data } | Chunk::Segment { index: key, data })
                if *pt == t && *key >= 1 =>
            {
                let e = out.entry(*r).or_default().entry(*f).or_default();
                if *key == 1 {
                    e.clear();
                }
                e.push_str(&text(data));
            }
            _ => {}
        }
    })?;
    Ok(out)
}
