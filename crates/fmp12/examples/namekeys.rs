//! Prints the name indexes of a file (`[cat].[1].[1]`: collation key → id)
//! next to the names they index: `name TAB key-hex`.
//! `cargo run -p fmp12 --example namekeys -- <file.fmp12>…`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    for f in std::env::args().skip(1) {
        let file = fmp12::File::open(&f).unwrap();
        // Catalog path (without the trailing [1]/[5]) → id → name.
        let mut names: BTreeMap<(Vec<u32>, u32), String> = BTreeMap::new();
        let mut keys: Vec<(Vec<u32>, Vec<u8>, u32)> = Vec::new();
        file.walk(|path, c| {
            let Some(p) = fmp12::int_path(path) else {
                return;
            };
            match c {
                fmp12::Chunk::Kv { key: 16, data } if p.len() >= 2 && p[p.len() - 2] == 5 => {
                    let cat = p[..p.len() - 2].to_vec();
                    names.insert((cat, p[p.len() - 1]), fmp12::text(data));
                }
                fmp12::Chunk::KvLong { key, data }
                    if p.len() >= 2 && p[p.len() - 2..] == [1, 1] =>
                {
                    let cat = p[..p.len() - 2].to_vec();
                    // Value: TO reference, or length + path integer.
                    let id = fmp12::graph::to_ref(data)
                        .or_else(|| data.get(1..).and_then(fmp12::path_int))
                        .unwrap_or(0);
                    keys.push((cat, key.to_vec(), id));
                }
                _ => {}
            }
        })
        .unwrap();
        for (cat, key, id) in keys {
            // TO ids in the index are 128 + n; the catalog stores them as path ints too.
            if let Some(n) = names.get(&(cat.clone(), id)) {
                println!("{n}\t{}", hex(&key));
            }
        }
    }
}
