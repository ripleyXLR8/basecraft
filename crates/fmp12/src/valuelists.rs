//! Value lists (`[33]`, FORMAT.md §9).

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
        /// Also display values from a second field (occurrence, field).
        second: Option<(u32, u32)>,
        /// Show only the second field's values.
        second_only: bool,
        /// Sort by the second field.
        sort_by_second: bool,
        /// Include only values related from this occurrence.
        related_from: Option<u32>,
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

/// Option bits of the definition (bytes 1–2).
pub const RELATED_ONLY: u16 = 0x01;
pub const SECOND_ONLY: u16 = 0x04;
pub const SORT_BY_SECOND: u16 = 0x02;

/// Definition (key 2): source (0 custom, 1 field), u16 options, first
/// field (`len TO | len 01 field | u16 repetition`), second field (same,
/// `02 01 00` when none), the occurrence related values start from, two
/// empty references, `01 7A`.
pub fn definition(source: &Source) -> Vec<u8> {
    let none = [2u8, 1, 0];
    let field = |f: Option<(u32, u32)>| -> Vec<u8> {
        match f {
            Some((to, f)) => [crate::graph::len_to(to), crate::graph::fref(f)].concat(),
            None => [none, none].concat(),
        }
    };
    let (kind, flags, first, second, from) = match source {
        Source::Field {
            to,
            field: f,
            second,
            second_only,
            sort_by_second,
            related_from,
        } => (
            1u8,
            related_from.map_or(0, |_| RELATED_ONLY)
                | if *second_only { SECOND_ONLY } else { 0 }
                | if *sort_by_second { SORT_BY_SECOND } else { 0 },
            Some((*to, *f)),
            *second,
            *related_from,
        ),
        _ => (0, 0, None, None, None),
    };
    let mut b = vec![kind];
    b.extend(flags.to_be_bytes());
    b.extend(field(first));
    b.extend([0, 1]);
    b.extend(field(second));
    b.extend([0, 1]);
    b.extend(from.map_or(none.to_vec(), crate::graph::len_to));
    b.extend(none);
    b.extend(none);
    b.extend([1, 0x7A]);
    b
}

/// (occurrence, field).
type FieldRef = (u32, u32);

/// Reads a field reference pair `len TO | len 01 field`; `None` when empty.
fn pair(b: &[u8]) -> Option<(Option<FieldRef>, &[u8])> {
    let tl = 1 + *b.first()? as usize;
    let to = to_ref(b.get(1..tl)?);
    let rest = b.get(tl..)?;
    let fl = 1 + *rest.first()? as usize;
    let field = rest.get(2..fl).and_then(path_int);
    Some((to.zip(field), rest.get(fl..)?))
}

fn parse(raw: &[u8], custom: Option<&Vec<u8>>) -> Source {
    match raw.first() {
        Some(0) => Source::Custom(custom.map(|t| text(t)).unwrap_or_default()),
        Some(1) => {
            let flags = raw
                .get(1..3)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
            let parsed = (|| {
                let (first, rest) = pair(raw.get(3..)?)?;
                let (second, rest) = pair(rest.get(2..)?)?;
                let rest = rest.get(2..)?;
                let from = to_ref(rest.get(1..1 + *rest.first()? as usize)?);
                Some((first?, second, from))
            })();
            match parsed {
                Some(((to, field), second, from)) => Source::Field {
                    to,
                    field,
                    second,
                    second_only: flags & SECOND_ONLY != 0,
                    sort_by_second: flags & SORT_BY_SECOND != 0,
                    related_from: from.filter(|_| flags & RELATED_ONLY != 0),
                },
                None => Source::Unknown(1),
            }
        }
        n => Source::Unknown(n.copied().unwrap_or(0)),
    }
}

/// A value list from the keys of its node `[33].[5].[V]`.
pub fn from_keys(id: u32, kv: &BTreeMap<u32, Vec<u8>>) -> ValueList {
    let raw = kv.get(&2).cloned().unwrap_or_default();
    ValueList {
        id,
        name: kv.get(&16).map(|t| text(t)).unwrap_or_default(),
        source: parse(&raw, kv.get(&3)),
        raw,
    }
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
        .map(|(id, kv)| from_keys(id, &kv))
        .collect())
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
    fn definitions_match_filemaker() {
        // Written by FileMaker Pro 26.
        for (h, src) in [
            (
                "00000002010002010000010201000201000001020100020100020100017a",
                Source::Custom(String::new()),
            ),
            (
                "0100060403d0000102010600010403d000010201070001020100020100020100017a",
                Source::Field {
                    to: 129,
                    field: 6,
                    second: Some((129, 7)),
                    second_only: true,
                    sort_by_second: true,
                    related_from: None,
                },
            ),
            (
                "0100010403d00002020101000102010002010000010403d00001020100020100017a",
                Source::Field {
                    to: 130,
                    field: 1,
                    second: None,
                    second_only: false,
                    sort_by_second: false,
                    related_from: Some(129),
                },
            ),
        ] {
            assert_eq!(parse(&hex(h), None), src);
            assert_eq!(definition(&src), hex(h));
        }
    }
}
