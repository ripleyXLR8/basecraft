//! Calculation compiler against every calculation stored in the FileMaker
//! starter templates, scripts and layouts included (skipped when they are
//! not installed; override the directory with FMP12_TEMPLATES):
//!
//! * key 6 derived from key 5 ([`calc::compiled`]) equals FileMaker's key 6;
//! * key 5 decompiled to text and compiled again gives the same keys 5 and 6.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use fmp12::{Chunk, calc};

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

#[test]
fn template_calculations_compile() {
    let (mut pairs, mut ok6, mut ok5, mut n5) = (0, 0, 0, 0);
    let mut fails: Vec<String> = Vec::new();
    for path in template_files() {
        let file = fmp12::File::open(&path).unwrap();
        let names = calc::FileNames::load(&file).unwrap();
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
                b => match calc::context(b) {
                    Some(t) => t,
                    None => continue,
                },
            };
            // Only real calculations (a decodable token stream).
            let Ok(text) = calc::decompile(&k5, context, &names) else {
                continue;
            };
            pairs += 1;
            match calc::compiled(&k5) {
                Ok(c) if c == k6 => ok6 += 1,
                r => {
                    if fails.len() < 30 {
                        fails.push(format!("k6 {p:?} {text:?}: {r:02x?}\n   want {k6:02x?}"));
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
            match calc::compile(&text, context, &names) {
                Ok(c) if c.tokens == k5 && c.compiled == k6 => ok5 += 1,
                // Our SCSU encoder picks other windows than FileMaker for
                // non-Latin scripts: same text, other bytes.
                Ok(c)
                    if !text.is_ascii()
                        && calc::decompile(&c.tokens, context, &names).as_ref() == Ok(&text) =>
                {
                    ok5 += 1
                }
                r => {
                    if fails.len() < 30 {
                        fails.push(format!(
                            "k5 {p:?} {text:?}: {r:02x?}\n   want {k5:02x?}\n   k6 {k6:02x?}"
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

/// Pairs from the templates, and the script compiler's old checks.
#[test]
fn compiled_from_key5() {
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    for (k5, k6) in [
        ("9b0c13017a00040c13017a009c3e0c13017a0005", "9b3e"),
        (
            "87040c13017a0016040403d0000402010b000105",
            "87000117040403d0000402010b0001",
        ),
        (
            "1a027e220c13017a00500c13017a0013012300",
            "501a027e2213012300",
        ),
        (
            "fa00441002000100100000000000000000000000000020",
            "44fa00100200010010000000000000000000000000",
        ),
    ] {
        assert_eq!(calc::compiled(&hex(k5)).unwrap(), hex(k6), "{k5}");
    }
}
