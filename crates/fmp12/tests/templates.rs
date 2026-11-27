//! Parses every .fmp12 starter template shipped with a local FileMaker Pro
//! install. Those files are Claris property and never committed; the test is
//! a no-op when they are absent (e.g. on CI). Override with FMP12_TEMPLATES.

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

/// Every `<dir>/<template>/<lang>.fmp12`.
fn template_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for t in std::fs::read_dir(dir).unwrap() {
        let Ok(files) = std::fs::read_dir(t.unwrap().path()) else {
            continue;
        };
        for f in files {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "fmp12") {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

fn template_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FMP12_TEMPLATES")
        .map(PathBuf::from)
        .unwrap_or_else(|| r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates".into());
    dir.is_dir().then_some(dir)
}

#[test]
fn all_templates_parse() {
    let Some(dir) = template_dir() else {
        eprintln!("no FileMaker templates found, skipping");
        return;
    };
    let mut n = 0;
    for f in template_files(&dir) {
        {
            let file = fmp12::File::open(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            file.walk(|_, _| {})
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let tables = fmp12::schema::tables(&file).unwrap();
            assert!(!tables.is_empty(), "{}: no tables", f.display());
            assert!(
                tables
                    .iter()
                    .all(|t| !t.name.is_empty() && !t.fields.is_empty())
            );
            n += 1;
        }
    }
    assert!(n > 0);
    eprintln!("{n} templates parsed");
}

#[test]
fn contacts_schema_en() {
    let Some(dir) = template_dir() else { return };
    let file = fmp12::File::open(dir.join("Contacts").join("en.fmp12")).unwrap();
    let tables = fmp12::schema::tables(&file).unwrap();
    let names: Vec<_> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names[..4],
        ["Contacts", "Addresses", "Phone Numbers", "Email Addresses"]
    );
    assert_eq!(tables[0].fields[&1].name, "First Name");
    assert_eq!(file.creator(), "Pro 12.0");
}

#[test]
fn summary_fields_and_break_fields() {
    use fmp12::schema::{Summary, SummaryOp};
    let Some(dir) = template_dir() else { return };
    let summary = |tpl: &str, table: &str, field: &str| {
        let file = fmp12::File::open(dir.join(tpl).join("en.fmp12")).unwrap();
        let tables = fmp12::schema::tables(&file).unwrap();
        let t = tables.iter().find(|t| t.name == table).unwrap();
        let f = t.fields.values().find(|f| f.name == field).unwrap();
        let s = f.summary().unwrap();
        (s, t.fields[&s.field].name.clone())
    };
    let (s, target) = summary("Assets", "Assets", "Total");
    assert_eq!((s.op, target.as_str()), (SummaryOp::Total, "Price"));
    let (s, target) = summary("Meetings", "Topics", "Number of Topics");
    assert_eq!((s.op, target.as_str()), (SummaryOp::Count, "Subject"));
    let bytes = fmp12::schema::Field::summary_bytes(Summary {
        op: SummaryOp::Maximum,
        field: 5,
    });
    let f = fmp12::schema::Field {
        options: vec![3, 2],
        summary_raw: bytes,
        ..Default::default()
    };
    assert_eq!(f.summary().unwrap().op, SummaryOp::Maximum);

    // Contact List: leading sub-summary by Contacts::Initial.
    let file = fmp12::File::open(dir.join("Contacts").join("en.fmp12")).unwrap();
    let tables = fmp12::schema::tables(&file).unwrap();
    let l = fmp12::layouts::layouts(&file)
        .unwrap()
        .into_iter()
        .find(|l| l.name == "Contact List")
        .unwrap();
    let p = l
        .parts
        .iter()
        .find(|p| p.kind == fmp12::layouts::part::LEADING_SUB_SUMMARY)
        .unwrap();
    let (to, field) = p.break_field.unwrap();
    assert_eq!(to as u64, l.table_occurrence & 0xFFF);
    assert_eq!(tables[0].fields[&field].name, "Initial");
}

#[test]
fn rewrite_preserves_content() {
    let Some(dir) = template_dir() else { return };
    for name in ["Contacts", "Tasks", "Assets"] {
        let file = fmp12::File::open(dir.join(name).join("en.fmp12")).unwrap();
        let before = fmp12::write::entries(&file).unwrap();
        let bytes = fmp12::write::rewrite(&file).unwrap();
        let again = fmp12::File::from_bytes(bytes).unwrap();
        let after = fmp12::write::entries(&again).unwrap();
        assert_eq!(before.len(), after.len(), "{name}");
        assert!(before == after, "{name}: content differs after rewrite");
        assert_eq!(
            fmp12::schema::tables(&again).unwrap(),
            fmp12::schema::tables(&file).unwrap()
        );
        eprintln!(
            "{name}: {} entries, {} → {} sectors",
            before.len(),
            file.sector_count(),
            again.sector_count()
        );
    }
}

/// FileMaker's own index sectors decode and re-encode byte for byte.
#[test]
fn index_sectors_reencode_byte_for_byte() {
    let Some(dir) = template_dir() else { return };
    let mut n = 0;
    for f in template_files(&dir) {
        let file = fmp12::File::open(&f).unwrap();
        for id in 1..=file.sector_count() {
            let s = file.sector(id);
            if s.level == 0 || s.deleted {
                continue;
            }
            let h = &file.bytes()[id as usize * fmp12::SECTOR..];
            let free = u16::from_be_bytes([h[14], h[15]]) as usize;
            let orig = &s.payload[..s.payload.len() - free];
            let seps = fmp12::write::decode_index(orig).unwrap();
            let again = fmp12::write::encode_index(&seps);
            assert!(again == orig, "{} sector {id}", f.display());
            n += 1;
        }
    }
    assert!(n > 0);
    eprintln!("{n} index sectors re-encoded");
}

/// Every FileMaker leaf re-packs to its own bytes, except the pops it may
/// end with (they depend on the next sector, which `pack` is not given).
#[test]
fn leaf_sectors_reencode_byte_for_byte() {
    let Some(dir) = template_dir() else { return };
    let mut n = 0;
    for f in template_files(&dir) {
        let file = fmp12::File::open(&f).unwrap();
        for s in file.leaves() {
            let h = &file.bytes()[s.id as usize * fmp12::SECTOR..];
            let free = u16::from_be_bytes([h[14], h[15]]) as usize;
            let orig = &s.payload[..s.payload.len() - free];
            let es = fmp12::write::leaf_entries(orig).unwrap();
            let pages = fmp12::write::pack(&es);
            assert_eq!(pages.len(), 1);
            let p = &pages[0].0;
            let same = orig.starts_with(p) && orig[p.len()..].iter().all(|b| *b == 0x40);
            assert!(same, "{} sector {}", f.display(), s.id);
            n += 1;
        }
    }
    assert!(n > 0);
    eprintln!("{n} leaf sectors re-encoded");
}

#[test]
fn locales() {
    use fmp12::schema::DateOrder::*;
    let Some(dir) = template_dir() else { return };
    let l = |lang: &str| {
        let f = fmp12::File::open(dir.join("Meetings").join(format!("{lang}.fmp12"))).unwrap();
        fmp12::schema::locale(&f).unwrap()
    };
    let en = l("en");
    assert_eq!(
        (en.language, en.date_order, en.am.as_str()),
        (21, Mdy, " AM")
    );
    assert_eq!(en.pm, " PM");
    let fr = l("fr");
    assert_eq!(
        (fr.date_order, fr.decimal, fr.thousands),
        (Dmy, ',', '\u{A0}')
    );
    assert!(fr.am.is_empty());
    let de = l("de");
    assert_eq!((de.date_order, de.date_separator), (Dmy, '.'));
    assert_eq!(l("ja").date_order, Ymd);
    assert_eq!((l("sv").date_order, l("sv").date_separator), (Ymd, '-'));
}

/// `write::sort_key` reproduces FileMaker's store order on every template.
#[test]
fn entries_are_in_store_order() {
    let Some(dir) = template_dir() else { return };
    for f in template_files(&dir) {
        let file = fmp12::File::open(&f).unwrap();
        let mut es = fmp12::write::entries(&file).unwrap();
        fmp12::write::drop_leftovers(&mut es);
        for w in es.windows(2) {
            let (a, b) = (fmp12::write::sort_key(&w[0]), fmp12::write::sort_key(&w[1]));
            assert!(a <= b, "{}: {:02x?} > {:02x?}", f.display(), w[0], w[1]);
        }
    }
}

/// Every calculation of every template field (calculated fields, auto-enter
/// and validation calculations): compiling its decompiled text gives back
/// FileMaker's token stream (key 5) and compiled form (key 6).
#[test]
fn field_calculations_recompile() {
    use std::collections::BTreeMap;
    let Some(dir) = template_dir() else {
        eprintln!("no FileMaker templates found, skipping");
        return;
    };
    let (mut ok, mut total, mut fails) = (0, 0, Vec::new());
    for f in template_files(&dir) {
        let file = fmp12::File::open(&f).unwrap();
        let names = fmp12::calc::FileNames::load(&file).unwrap();
        // Nodes under a field definition: key → value (long values sit in
        // sub-nodes [5] / [6], keys 1…n).
        let mut kv: BTreeMap<Vec<u32>, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
        file.walk(|path, c| {
            let (Some(p), fmp12::Chunk::Kv { key, data }) = (fmp12::int_path(path), c) else {
                return;
            };
            if p.len() >= 5 && p[0] >= 128 && p[1..3] == [3, 5] {
                kv.entry(p)
                    .or_default()
                    .entry(*key)
                    .or_default()
                    .extend_from_slice(data);
            }
        })
        .unwrap();
        for (p, m) in &kv {
            let Some(k4) = m.get(&4) else { continue };
            let long = |k: u32| {
                let mut q = p.clone();
                q.push(k);
                kv.get(&q).map(|s| {
                    s.range(1..)
                        .flat_map(|(_, v)| v.clone())
                        .collect::<Vec<u8>>()
                })
            };
            let k5 = m.get(&5).cloned().or_else(|| long(5));
            let k6 = m.get(&6).cloned().or_else(|| long(6));
            let (Some(k5), Some(k6)) = (k5, k6) else {
                continue;
            };
            let ctx = fmp12::calc::context(k4).unwrap_or(0);
            total += 1;
            let src = fmp12::calc::decompile(&k5, ctx, &names).unwrap();
            let c = fmp12::calc::compile(&src, ctx, &names)
                .unwrap_or_else(|e| panic!("{}: {src:?}: {e}", f.display()));
            let exotic = src
                .chars()
                .any(|c| c as u32 >= 0x100 && !(0x2000..0x2080).contains(&(c as u32)));
            let same_text =
                fmp12::calc::decompile(&c.tokens, ctx, &names).as_deref() == Ok(src.as_str());
            if c.tokens == k5 && c.compiled == k6 || exotic && same_text {
                ok += 1;
            } else {
                fails.push(format!(
                    "{}: {src:?}\n  {:02x?}\n  {:02x?}",
                    f.display(),
                    c.compiled,
                    k6
                ));
            }
        }
    }
    for x in fails.iter().take(10) {
        eprintln!("{x}");
    }
    eprintln!("{ok}/{total} template field calculations recompile");
    assert!(total > 0 && fails.is_empty());
}
