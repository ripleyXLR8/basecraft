//! Calculation compiler against every calculation stored in the FileMaker
//! starter templates (skipped when they are not installed; override the
//! directory with FMP12_TEMPLATES):
//!
//! * key 6 derived from key 5 equals FileMaker's key 6;
//! * key 5 decompiled to text and compiled again gives the same bytes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use fmp12::calc_compile::{Resolve, compiled, source_tokens};
use fmp12::{Chunk, schema};

fn template_files() -> Vec<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    let mut out = Vec::new();
    for t in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(t.path()).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|e| e == "fmp12") {
                out.push(f.path());
            }
        }
    }
    out.sort();
    out
}

pub struct FileNames {
    tables: Vec<schema::Table>,
    tos: Vec<fmp12::graph::TableOccurrence>,
}

impl fmp12::calc::Names for FileNames {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        let bt = self.tos.iter().find(|t| t.id == to)?.base_table;
        let t = self.tables.iter().find(|t| t.id == bt)?;
        Some(t.fields.get(&field)?.name.clone())
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(self.tos.iter().find(|t| t.id == to)?.name.clone())
    }
}

struct Ctx<'a>(&'a FileNames, u32);

impl Resolve for Ctx<'_> {
    fn field(&self, to: Option<&str>, name: &str) -> Option<(u32, u32)> {
        let o = match to {
            Some(n) => self.0.tos.iter().find(|t| t.name == n)?,
            None => self.0.tos.iter().find(|t| t.id == self.1)?,
        };
        let t = self.0.tables.iter().find(|t| t.id == o.base_table)?;
        let f = t.fields.iter().find(|(_, f)| f.name == name)?;
        Some((o.id, *f.0))
    }
}

#[test]
fn template_calculations_compile() {
    let (mut pairs, mut ok6, mut ok5, mut n5) = (0, 0, 0, 0);
    let mut fails: Vec<String> = Vec::new();
    for path in template_files() {
        let file = fmp12::File::open(&path).unwrap();
        let names = FileNames {
            tables: schema::tables(&file).unwrap(),
            tos: fmp12::graph::table_occurrences(&file).unwrap(),
        };
        // Calculation nodes: path → keys 4, 5, 6.
        let mut nodes: BTreeMap<Vec<u32>, [Option<Vec<u8>>; 3]> = BTreeMap::new();
        file.walk(|p, c| {
            let (Some(p), Chunk::Kv { key, data }) = (fmp12::int_path(p), c) else {
                return;
            };
            if (4..=6).contains(key) {
                nodes.entry(p).or_default()[*key as usize - 4] = Some(data.to_vec());
            }
        })
        .unwrap();
        for (p, [k4, k5, k6]) in nodes {
            let (Some(k4), Some(k5), Some(k6)) = (k4, k5, k6) else {
                continue;
            };
            let context = match k4.as_slice() {
                [1, 0] => 0,
                b => match fmp12::graph::to_ref(b) {
                    Some(t) => t,
                    None => continue,
                },
            };
            // Only real calculations (a decodable token stream).
            let Ok(text) = fmp12::calc::decompile(&k5, context, &names) else {
                continue;
            };
            pairs += 1;
            match compiled(&k5) {
                Ok(c) if c == k6 => ok6 += 1,
                r => {
                    if fails.len() < 30 {
                        fails.push(format!(
                            "k6 {p:?} {text:?}: {r:02x?}
   want {k6:02x?}"
                        ));
                    }
                }
            }
            if text.contains("<Field Missing>")
                || text.contains("<Table Missing>")
                || text.contains("( ::")
                || text.contains("(::")
            {
                continue;
            }
            n5 += 1;
            match source_tokens(&text, context, &Ctx(&names, context)) {
                Ok(t) if t == k5 => ok5 += 1,
                // Our SCSU encoder picks other windows than FileMaker for
                // non-Latin scripts: same text, other bytes.
                Ok(t)
                    if !text.is_ascii()
                        && fmp12::calc::decompile(&t, context, &names).as_ref() == Ok(&text) =>
                {
                    ok5 += 1
                }
                r => {
                    if fails.len() < 30 {
                        fails.push(format!(
                            "k5 {p:?} {text:?}: {r:02x?}
   want {k5:02x?}"
                        ));
                    }
                }
            }
        }
    }
    eprintln!("{pairs} calculations: key 6 {ok6} ok, key 5 round trip {ok5}/{n5}");
    for f in &fails {
        eprintln!("{f}");
    }
    assert_eq!(ok6, pairs);
    assert_eq!(ok5, n5);
}
