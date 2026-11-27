//! Loads the FileMaker templates (skipped when absent) and exercises the engine.

use engine::{Database, Order, Request, find, sort};
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    let d = PathBuf::from(r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates");
    d.is_dir().then_some(d)
}

#[test]
fn calculated_fields_parse_everywhere() {
    let Some(d) = dir() else { return };
    let mut n = 0;
    for t in [
        "Contacts",
        "Tasks",
        "Assets",
        "Inventory",
        "Meetings",
        "Content_Management",
    ] {
        let db = Database::open(d.join(t).join("en.fmp12")).unwrap();
        for tbl in &db.tables {
            for f in tbl
                .fields
                .iter()
                .filter(|f| f.kind == fmp12::schema::FieldKind::Calculated)
            {
                let src = f
                    .calc
                    .as_ref()
                    .unwrap_or_else(|| panic!("{t}::{}: no calc", f.name));
                calc::parse(src).unwrap_or_else(|e| panic!("{t}::{}: {src:?}: {e:?}", f.name));
                n += 1;
            }
        }
    }
    assert!(n > 0);
}

#[test]
fn records_find_sort_and_calculate() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let t = db.tables.iter().position(|t| t.name == "Contacts").unwrap();
    let tid = db.tables[t].id;
    let first = db.tables[t].field_by_name("First Name").unwrap().id;
    let last = db.tables[t].field_by_name("Last Name").unwrap().id;
    let initial = db.tables[t].field_by_name("Initial").unwrap().id;
    for (f, l) in [("Ada", "Lovelace"), ("Alan", "Turing"), ("Grace", "Hopper")] {
        let r = db.tables[t].new_record();
        db.tables[t].records[r].values.insert(first, f.into());
        db.tables[t].records[r].values.insert(last, l.into());
    }
    // Initial = Case ( not IsEmpty ( Last Name ) ; Left ( Last Name ; 1 ) ; … )
    assert_eq!(db.value(tid, 1, initial).to_text(), "T");
    let req = Request {
        criteria: vec![(first, "a".into())],
        omit: false,
    };
    let mut found = find(&db, tid, &[req]);
    assert_eq!(found, vec![0, 1]);
    sort(&db, tid, &mut found, &[(last, Order::Descending)]);
    assert_eq!(found, vec![1, 0]);
}
