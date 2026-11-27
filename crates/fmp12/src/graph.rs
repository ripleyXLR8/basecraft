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

/// Join operator of a relationship predicate (FORMAT.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// Cartesian product (×): every record matches.
    Cartesian,
    Unknown(u8),
}

impl JoinOp {
    pub fn from_code(c: u8) -> JoinOp {
        use JoinOp::*;
        match c {
            0 => Equal,
            1 => NotEqual,
            2 => Less,
            3 => LessOrEqual,
            4 => Greater,
            5 => GreaterOrEqual,
            6 => Cartesian,
            n => Unknown(n),
        }
    }

    pub fn code(self) -> u8 {
        use JoinOp::*;
        match self {
            Equal => 0,
            NotEqual => 1,
            Less => 2,
            LessOrEqual => 3,
            Greater => 4,
            GreaterOrEqual => 5,
            Cartesian => 6,
            Unknown(n) => n,
        }
    }

    /// The symbol FileMaker shows: `=`, `≠`, `<`, `≤`, `>`, `≥`, `×`.
    pub fn symbol(self) -> &'static str {
        use JoinOp::*;
        match self {
            Equal => "=",
            NotEqual => "≠",
            Less => "<",
            LessOrEqual => "≤",
            Greater => ">",
            GreaterOrEqual => "≥",
            Cartesian => "×",
            Unknown(_) => "?",
        }
    }

    /// Reads a symbol (also the ASCII spellings `!=`, `<>`, `<=`, `>=`, `x`, `*`).
    pub fn parse(s: &str) -> Option<JoinOp> {
        use JoinOp::*;
        Some(match s.trim() {
            "=" | "==" => Equal,
            "≠" | "!=" | "<>" => NotEqual,
            "<" => Less,
            "≤" | "<=" => LessOrEqual,
            ">" => Greater,
            "≥" | ">=" => GreaterOrEqual,
            "×" | "x" | "X" | "*" => Cartesian,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub op: JoinOp,
    pub left_field: u32,
    pub right_field: u32,
}

/// One key of a relationship's sort order: a field of the sorted side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// Table occurrence of the field (the sorted side).
    pub to: u32,
    pub field: u32,
    pub descending: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relationship {
    pub id: u32,
    pub left: u32,
    pub right: u32,
    pub predicates: Vec<Predicate>,
    /// Sort order of the left side's records (empty: unsorted).
    pub left_sort: Vec<SortKey>,
    /// Sort order of the right side's records.
    pub right_sort: Vec<SortKey>,
    pub raw: Vec<u8>,
}

/// Option flags, the last byte of key 2 (FORMAT.md §7).
pub const RIGHT_DELETE: u8 = 0x01;
pub const LEFT_DELETE: u8 = 0x02;
pub const RIGHT_CREATE: u8 = 0x10;
pub const LEFT_CREATE: u8 = 0x20;
pub const RIGHT_SORT: u8 = 0x40;
pub const LEFT_SORT: u8 = 0x80;

impl Relationship {
    pub fn flags(&self) -> u8 {
        self.raw.last().copied().unwrap_or(0)
    }
    pub fn left_cascade_create(&self) -> bool {
        self.flags() & LEFT_CREATE != 0
    }
    pub fn right_cascade_create(&self) -> bool {
        self.flags() & RIGHT_CREATE != 0
    }
    /// Left records are deleted when a related right record is deleted.
    pub fn left_cascade_delete(&self) -> bool {
        self.flags() & LEFT_DELETE != 0
    }
    pub fn right_cascade_delete(&self) -> bool {
        self.flags() & RIGHT_DELETE != 0
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

/// A join predicate (`[3].[251].[5].[R].[3]` keys 1…n): `op | len 01 field | len 01 field`.
pub fn parse_predicate(b: &[u8]) -> Option<Predicate> {
    let op = JoinOp::from_code(*b.first()?);
    let (left_field, rest) = field_ref(&b[1..])?;
    let (right_field, _) = field_ref(rest)?;
    Some(Predicate {
        op,
        left_field,
        right_field,
    })
}

/// Field reference `len 01 path-int`.
pub(crate) fn fref(f: u32) -> Vec<u8> {
    let e = crate::save::path_elem(f);
    let mut v = vec![1 + e.len() as u8, 1];
    v.extend(e);
    v
}

/// Table occurrence reference `len 03 D0 hi lo`.
pub(crate) fn len_to(to: u32) -> Vec<u8> {
    let [hi, lo] = (to.saturating_sub(128) as u16).to_be_bytes();
    vec![4, 3, 0xD0, hi, lo]
}

pub fn predicate_bytes(p: &Predicate) -> Vec<u8> {
    [vec![p.op.code()], fref(p.left_field), fref(p.right_field)].concat()
}

/// A sort order: u32 count, then per key `len TO | len 01 field | 00 01 |
/// 02 01 00 | 02 01 00 | 00 01 | value list (00: none) | order | u32 0`.
fn sort_bytes(keys: &[SortKey]) -> Vec<u8> {
    let mut b = (keys.len() as u32).to_be_bytes().to_vec();
    for k in keys {
        b.extend(len_to(k.to));
        b.extend(fref(k.field));
        b.extend([
            0,
            1,
            2,
            1,
            0,
            2,
            1,
            0,
            0,
            1,
            0,
            k.descending as u8,
            0,
            0,
            0,
            0,
        ]);
    }
    b
}

/// Reads a sort order at the start of `b`; returns it and the rest.
fn parse_sort(b: &[u8]) -> Option<(Vec<SortKey>, &[u8])> {
    let n = u32::from_be_bytes(b.get(..4)?.try_into().ok()?);
    let mut rest = &b[4..];
    let mut keys = Vec::new();
    // Length-prefixed item.
    let item = |r: &[u8]| -> Option<usize> { Some(1 + *r.first()? as usize) };
    for _ in 0..n {
        let tl = item(rest)?;
        let to = to_ref(rest.get(1..tl)?)?;
        rest = rest.get(tl..)?;
        let (field, r) = field_ref(rest)?;
        rest = r.get(2..)?; // repetition
        for _ in 0..2 {
            rest = rest.get(item(rest)?..)?;
        }
        rest = rest.get(2..)?;
        rest = rest.get(item(rest)?..)?; // value list
        let descending = *rest.first()? == 1;
        rest = rest.get(5..)?;
        keys.push(SortKey {
            to,
            field,
            descending,
        });
    }
    Some((keys, rest))
}

/// Key 2 of a relationship: `04 TOref (left) 04 TOref (right) | right
/// side's sort order | left side's sort order | 00 00 00 | flags`.
pub fn relationship_bytes(
    left: u32,
    right: u32,
    left_sort: &[SortKey],
    right_sort: &[SortKey],
    flags: u8,
) -> Vec<u8> {
    let mut b = len_to(left);
    b.extend(len_to(right));
    b.extend(sort_bytes(right_sort));
    b.extend(sort_bytes(left_sort));
    b.extend([0, 0, 0, flags]);
    b
}

/// Fills a relationship from its key 2.
pub fn read_relationship(r: &mut Relationship, data: &[u8]) {
    r.raw = data.to_vec();
    r.left = data.get(1..5).and_then(to_ref).unwrap_or(0);
    r.right = data.get(6..10).and_then(to_ref).unwrap_or(0);
    if let Some((right, rest)) = data.get(10..).and_then(parse_sort)
        && let Some((left, _)) = parse_sort(rest)
    {
        r.right_sort = right;
        r.left_sort = left;
    }
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
            2 => read_occurrence(to, data),
            _ => {}
        }
    })?;
    Ok(out.into_values().collect())
}

/// Fills an occurrence from its key 2: `02 01 00 | len, 02, path-int base
/// table | f32 left, top, width, height | u32 | u32 | rgba`.
pub fn read_occurrence(to: &mut TableOccurrence, d: &[u8]) {
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
    if let Some(x) = d.len().checked_sub(4).and_then(|i| d.get(i..)) {
        to.rgba = [x[0], x[1], x[2], x[3]];
    }
    to.raw = d.to_vec();
}

pub fn relationships(file: &File) -> Result<Vec<Relationship>, Error> {
    let mut out: BTreeMap<u32, Relationship> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match (p.as_slice(), c) {
            ([3, 251, 5, id], Chunk::Kv { key: 2, data }) => {
                let r = out.entry(*id).or_default();
                r.id = *id;
                read_relationship(r, data);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn sorted_relationship_round_trips() {
        // Written by FileMaker Pro 26: Clients (129) – Invoices (130),
        // right sorted by Amount (desc) and Day, left by Name; create and
        // delete on the right.
        let b = hex(
            "0403d000010403d00002000000020403d00002020103000102010002010000010001000000000403d0000202010400010201000201000001000000000000000000010403d0000102010700010201000201000001000000000000000000d1",
        );
        let mut r = Relationship::default();
        read_relationship(&mut r, &b);
        assert_eq!((r.left, r.right), (129, 130));
        let k = |to, field, descending| SortKey {
            to,
            field,
            descending,
        };
        assert_eq!(r.right_sort, [k(130, 3, true), k(130, 4, false)]);
        assert_eq!(r.left_sort, [k(129, 7, false)]);
        assert!(r.right_cascade_create() && r.right_cascade_delete());
        assert!(!r.left_cascade_create() && !r.left_cascade_delete());
        assert_eq!(
            relationship_bytes(129, 130, &r.left_sort, &r.right_sort, 0xD1),
            b
        );
        // Unsorted, delete on the left.
        let b = hex("0403d000010403d00003000000000000000000000002");
        read_relationship(&mut r, &b);
        assert!(r.left_cascade_delete() && r.right_sort.is_empty());
        assert_eq!(relationship_bytes(129, 131, &[], &[], 2), b);
    }

    #[test]
    fn predicates_round_trip() {
        for (h, op) in [
            ("00020106020102", JoinOp::Equal),
            ("06020108020104", JoinOp::Cartesian),
            ("03020107020103", JoinOp::LessOrEqual),
        ] {
            let p = parse_predicate(&hex(h)).unwrap();
            assert_eq!(p.op, op);
            assert_eq!(predicate_bytes(&p), hex(h));
            assert_eq!(JoinOp::parse(op.symbol()), Some(op));
        }
    }
}
