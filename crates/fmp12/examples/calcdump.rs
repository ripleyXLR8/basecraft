//! Prints every calculation node of a file: path, source text, token stream
//! (key 5) and compiled form (key 6).
//! `cargo run -p fmp12 --example calcdump -- <file.fmp12>…`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

struct Names {
    tables: Vec<fmp12::schema::Table>,
    tos: Vec<fmp12::graph::TableOccurrence>,
}

impl fmp12::calc::Names for Names {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        let bt = self.tos.iter().find(|t| t.id == to)?.base_table;
        Some(
            self.tables
                .iter()
                .find(|t| t.id == bt)?
                .fields
                .get(&field)?
                .name
                .clone(),
        )
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(self.tos.iter().find(|t| t.id == to)?.name.clone())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    for f in std::env::args().skip(1) {
        let file = fmp12::File::open(&f).unwrap();
        let names = Names {
            tables: fmp12::schema::tables(&file).unwrap(),
            tos: fmp12::graph::table_occurrences(&file).unwrap(),
        };
        let mut kvs: BTreeMap<Vec<Vec<u8>>, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
        file.walk(|path, c| {
            let fmp12::Chunk::Kv { key, data } = c else {
                return;
            };
            let p: Vec<Vec<u8>> = path.iter().map(|x| x.to_vec()).collect();
            kvs.entry(p)
                .or_default()
                .entry(*key)
                .or_default()
                .extend_from_slice(data);
        })
        .unwrap();
        let mut calcs: BTreeMap<Vec<Vec<u8>>, [Vec<u8>; 3]> = BTreeMap::new();
        for (p, m) in &kvs {
            let Some(k4) = m.get(&4) else { continue };
            // Long values: keys 1…n of `[node].[5]` / `[node].[6]`.
            let long = |k: u8| {
                let mut q = p.clone();
                q.push(vec![k]);
                kvs.get(&q)
                    .map(|s| {
                        s.range(1..)
                            .flat_map(|(_, v)| v.clone())
                            .collect::<Vec<u8>>()
                    })
                    .unwrap_or_default()
            };
            let k5 = m.get(&5).cloned().unwrap_or_else(|| long(5));
            let k6 = m.get(&6).cloned().unwrap_or_else(|| long(6));
            calcs.insert(p.clone(), [k4.clone(), k5, k6]);
        }
        for (p, [k4, k5, k6]) in calcs {
            if k5.is_empty() || k6.is_empty() || !(k4 == [1, 0] || k4.len() == 4 && k4[0] == 3) {
                continue;
            }
            let ctx = fmp12::calc::context(&k4).unwrap_or(0);
            let src =
                fmp12::calc::decompile(&k5, ctx, &names).unwrap_or_else(|e| format!("<ERR {e}>"));
            let p: Vec<String> = p.iter().map(|x| hex(x)).collect();
            println!(
                "{}\t{}\t{}\t{}\t{}",
                p.join("."),
                hex(&k4),
                src.replace(['\r', '\n'], "¶").replace('\t', "→"),
                hex(&k5),
                hex(&k6)
            );
        }
    }
}
