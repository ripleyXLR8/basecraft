//! Relationship graph: table occurrences (`[3].[17]`) and relationships (`[3].[251]`).

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, path_int, text};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableOccurrence {
    /// Internal id (≥ 128). FileMaker's XML shows `0x104000 | id`.
    pub id: u32,
    pub name: String,
    /// Base table id.
    pub base_table: u32,
    pub top: f32,
    pub left: f32,
    pub width: f32,
    pub height: f32,
    /// Collapsed height shown in the graph.
    pub collapsed_height: u32,
    pub rgba: [u8; 4],
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOp {
    Equal,
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub op: JoinOp,
    pub left_field: u32,
    pub right_field: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relationship {
    pub id: u32,
    pub left: u32,
    pub right: u32,
    pub predicates: Vec<Predicate>,
    pub raw: Vec<u8>,
}

impl Relationship {
    fn flags(&self) -> u8 {
        self.raw.last().copied().unwrap_or(0)
    }
    pub fn left_cascade_create(&self) -> bool {
        self.flags() & 0x20 != 0
    }
    pub fn right_cascade_create(&self) -> bool {
        self.flags() & 0x10 != 0
    }
}

/// Decodes a table-occurrence reference `03 D0 hi lo` (id = 128 + hi:lo).
pub fn to_ref(b: &[u8]) -> Option<u32> {
    match b {
        [3, 0xD0, hi, lo] => Some(128 + u16::from_be_bytes([*hi, *lo]) as u32),
        _ => None,
    }
}

fn f32_at(b: &[u8], i: usize) -> f32 {
    b.get(i..i + 4)
        .map_or(0.0, |x| f32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

/// Field reference inside a join predicate: `len, 01, path-int field id`.
fn field_ref(b: &[u8]) -> Option<(u32, &[u8])> {
    let n = *b.first()? as usize;
    let r = b.get(1..1 + n)?;
    Some((path_int(r.get(1..)?)?, &b[1 + n..]))
}

fn parse_predicate(b: &[u8]) -> Option<Predicate> {
    let op = match b.first()? {
        0 => JoinOp::Equal,
        n => JoinOp::Unknown(*n),
    };
    let (left_field, rest) = field_ref(&b[1..])?;
    let (right_field, _) = field_ref(rest)?;
    Some(Predicate {
        op,
        left_field,
        right_field,
    })
}

pub fn table_occurrences(file: &File) -> Result<Vec<TableOccurrence>, Error> {
    let mut out: BTreeMap<u32, TableOccurrence> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        let ([3, 17, 5, id], Chunk::Kv { key, data }) = (p.as_slice(), c) else {
            return;
        };
        let to = out.entry(*id).or_insert_with(|| TableOccurrence {
            id: *id,
            ..Default::default()
        });
        match key {
            16 => to.name = text(data),
            2 => {
                let d = *data;
                // 02 01 00 | len, 02, path-int base table | f32 left, top, width, height | u32 | u32 | rgba
                let n = d.get(3).copied().unwrap_or(0) as usize;
                to.base_table = d.get(5..4 + n).and_then(path_int).unwrap_or(0);
                let f = 4 + n;
                to.left = f32_at(d, f);
                to.top = f32_at(d, f + 4);
                to.width = f32_at(d, f + 8);
                to.height = f32_at(d, f + 12);
                to.collapsed_height = d
                    .get(f + 16..f + 20)
                    .map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
                if let Some(x) = d.get(d.len() - 4..) {
                    to.rgba = [x[0], x[1], x[2], x[3]];
                }
                to.raw = d.to_vec();
            }
            _ => {}
        }
    })?;
    Ok(out.into_values().collect())
}

pub fn relationships(file: &File) -> Result<Vec<Relationship>, Error> {
    let mut out: BTreeMap<u32, Relationship> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match (p.as_slice(), c) {
            ([3, 251, 5, id], Chunk::Kv { key: 2, data }) => {
                let r = out.entry(*id).or_default();
                r.id = *id;
                r.raw = data.to_vec();
                // len 04 + TO ref, twice
                r.left = data.get(1..5).and_then(to_ref).unwrap_or(0);
                r.right = data.get(6..10).and_then(to_ref).unwrap_or(0);
            }
            ([3, 251, 5, id, 3], Chunk::Kv { data, .. }) => {
                if let Some(pr) = parse_predicate(data) {
                    out.entry(*id).or_default().predicates.push(pr);
                }
            }
            _ => {}
        }
    })?;
    Ok(out.into_values().collect())
}
