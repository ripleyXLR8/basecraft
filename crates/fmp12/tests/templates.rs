//! Parses every .fmp12 starter template shipped with a local FileMaker Pro
//! install. Those files are Claris property and never committed; the test is
//! a no-op when they are absent (e.g. on CI). Override with FMP12_TEMPLATES.

use std::path::PathBuf;

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
    for t in std::fs::read_dir(dir).unwrap() {
        let t = t.unwrap().path();
        let Ok(files) = std::fs::read_dir(&t) else {
            continue;
        };
        for f in files {
            let f = f.unwrap().path();
            if f.extension().is_none_or(|e| e != "fmp12") {
                continue;
            }
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
    assert_eq!(tables[0].fields[&1], "First Name");
    assert_eq!(file.creator(), "Pro 12.0");
}
