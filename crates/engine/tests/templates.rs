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

#[test]
fn related_records_follow_relationships() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let ti = |db: &Database, n: &str| db.tables.iter().position(|t| t.name == n).unwrap();
    let (c, p) = (ti(&db, "Contacts"), ti(&db, "Phone Numbers"));
    let pk = db.tables[c].field_by_name("PrimaryKey").unwrap().id;
    let fk = db.tables[p].field_by_name("ForeignKey").unwrap().id;
    let num = db.tables[p].field_by_name("Number").unwrap().id;
    for key in ["A", "B"] {
        let r = db.tables[c].new_record();
        db.tables[c].records[r].values.insert(pk, key.into());
    }
    for (key, n) in [("A", "111"), ("B", "222"), ("a", "333")] {
        let r = db.tables[p].new_record();
        db.tables[p].records[r].values.insert(fk, key.into());
        db.tables[p].records[r].values.insert(num, n.into());
    }
    let to = |db: &Database, n: &str| db.occurrences.iter().find(|o| o.name == n).unwrap().id;
    let (cto, pto) = (to(&db, "Contacts"), to(&db, "Phone Numbers"));
    // Text joins are case-insensitive, like FileMaker's English index.
    assert_eq!(db.related(cto, 0, pto), vec![0, 2]);
    assert_eq!(db.related(cto, 1, pto), vec![1]);
}

#[test]
fn custom_value_lists() {
    let Some(d) = dir() else { return };
    let db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let vl = db
        .value_lists
        .iter()
        .find(|v| v.name == "Phone Type")
        .unwrap();
    assert_eq!(
        db.value_list_values(vl.id),
        ["Home", "Mobile", "Work", "Fax"]
    );
}

#[test]
fn design_functions_and_sql() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let c = db.tables.iter().position(|t| t.name == "Contacts").unwrap();
    let (tid, first) = (
        db.tables[c].id,
        db.tables[c].field_by_name("First Name").unwrap().id,
    );
    for n in ["Ada", "Grace"] {
        let r = db.tables[c].new_record();
        db.tables[c].records[r].values.insert(first, n.into());
    }
    let to = db
        .occurrences
        .iter()
        .find(|o| o.name == "Contacts")
        .unwrap()
        .id;
    let ev = |s: &str| db.eval_source(s, tid, to, Some(0)).to_text();
    assert!(ev("TableNames ( \"\" )").contains("Addresses"));
    assert!(ev("ValueListItems ( \"\" ; \"Phone Type\" )").starts_with("Home\rMobile"));
    assert_eq!(
        ev(
            "ExecuteSQL ( \"SELECT \\\"First Name\\\" FROM Contacts ORDER BY \\\"First Name\\\" DESC\" ; \"\" ; \"\" )"
        ),
        "Grace\rAda"
    );
    assert_eq!(ev("GetNthRecord ( ${First Name} ; 2 )"), "Grace");
}
