//! Accounts (`[23].[1]`) and privilege sets (`[23].[2]`).
//!
//! Decoded: names, and each account's privilege set (key 11 = `01 id`).
//! Not yet decoded: enabled flag, password data (key 6 looks like a salted
//! digest), and the privilege bits of a set (keys 2, 10 and sub-path 9).

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, text};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub id: u32,
    pub name: String,
    /// Privilege set id.
    pub privilege_set: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivilegeSet {
    pub id: u32,
    /// e.g. "[Full Access]" for the built-in sets.
    pub name: String,
}

fn be(b: &[u8]) -> u32 {
    b.iter().fold(0, |a, x| (a << 8) | *x as u32)
}

pub fn accounts(file: &File) -> Result<Vec<Account>, Error> {
    let mut out: BTreeMap<u32, Account> = BTreeMap::new();
    file.walk(|path, c| {
        let (Some(p), Chunk::Kv { key, data }) = (int_path(path), c) else {
            return;
        };
        match p.as_slice() {
            [23, 1, 5, id] => {
                let a = out.entry(*id).or_default();
                a.id = *id;
                match key {
                    16 => a.name = text(data),
                    // `len, id`
                    11 => a.privilege_set = data.get(1..).map_or(0, be),
                    _ => {}
                }
            }
            // Long or localised names: keys 1…n of `[16]`.
            [23, 1, 5, id, 16] if *key >= 1 => {
                let a = out.entry(*id).or_default();
                a.id = *id;
                if *key == 1 {
                    a.name.clear();
                }
                a.name.push_str(&text(data));
            }
            _ => {}
        }
    })?;
    Ok(out
        .into_values()
        .map(|mut a| {
            a.name = localize(&a.name);
            a
        })
        .collect())
}

/// Built-in names are stored as private-use characters that FileMaker
/// translates on display (U+E002 = "Guest", as in the "[Guest]" account).
pub fn localize(s: &str) -> String {
    // ponytail: only the Guest token has been observed.
    s.replace('\u{e002}', "Guest")
}

pub fn privilege_sets(file: &File) -> Result<Vec<PrivilegeSet>, Error> {
    let mut out: BTreeMap<u32, PrivilegeSet> = BTreeMap::new();
    file.walk(|path, c| {
        if let (Some([23, 2, 5, id]), Chunk::Kv { key: 16, data }) = (int_path(path).as_deref(), c)
        {
            out.insert(
                *id,
                PrivilegeSet {
                    id: *id,
                    name: text(data),
                },
            );
        }
    })?;
    Ok(out.into_values().collect())
}
