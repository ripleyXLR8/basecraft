//! Parses every .fmp12 starter template shipped with a local FileMaker Pro
//! install. Those files are Claris property and never committed; the test is
//! a no-op when they are absent (e.g. on CI). Override with FMP12_TEMPLATES.

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
