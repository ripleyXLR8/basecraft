//! Value lists (`[33]`).

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, graph::to_ref, int_path, path_int, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Custom values, separated by CR.
    Custom(String),
    /// Values from a field: (table occurrence id, field id).
    Field {
        to: u32,
        field: u32,
    },
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueList {
    pub id: u32,
    pub name: String,
    pub source: Source,
    pub raw: Vec<u8>,
}

pub fn value_lists(file: &File) -> Result<Vec<ValueList>, Error> {
    let mut nodes: BTreeMap<u32, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
    file.walk(|path, c| {
        if let (Some([33, 5, id]), Chunk::Kv { key, data }) = (int_path(path).as_deref(), c) {
            nodes.entry(*id).or_default().insert(*key, data.to_vec());
        }
    })?;
    Ok(nodes
        .into_iter()
        .map(|(id, kv)| {
            let raw = kv.get(&2).cloned().unwrap_or_default();
            let source = match raw.first() {
                Some(0) => Source::Custom(kv.get(&3).map(|t| text(t)).unwrap_or_default()),
                // 01 00 flags | len 04 + TO ref | len, 01, path-int field
                Some(1) => {
                    let to = raw.get(4..8).and_then(to_ref).unwrap_or(0);
                    let n = raw.get(8).copied().unwrap_or(0) as usize;
                    let field = raw.get(10..9 + n).and_then(path_int).unwrap_or(0);
                    Source::Field { to, field }
                }
                n => Source::Unknown(n.copied().unwrap_or(0)),
            };
            ValueList {
                id,
                name: kv.get(&16).map(|t| text(t)).unwrap_or_default(),
                source,
                raw,
            }
        })
        .collect())
}
