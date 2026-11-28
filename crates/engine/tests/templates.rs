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

#[test]
fn new_record_applies_auto_enter() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let tid = db.tables.iter().find(|t| t.name == "Contacts").unwrap().id;
    let r = db.new_record(tid).unwrap();
    let t = db.table(tid).unwrap();
    let get = |n: &str| {
        t.records[r]
            .values
            .get(&t.field_by_name(n).unwrap().id)
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(get("PrimaryKey").len(), 36, "Get ( UUID ) auto-enter");
    assert_eq!(get("CreatedBy"), "Admin");
    assert!(!get("CreationTimestamp").is_empty());
    assert!(!get("ModificationTimestamp").is_empty());
}

#[test]
fn validation_on_commit() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Contacts").join("en.fmp12")).unwrap();
    let tid = db.tables.iter().find(|t| t.name == "Contacts").unwrap().id;
    let a = db.new_record(tid).unwrap();
    assert!(
        db.validate(tid, a).is_empty(),
        "auto-entered values satisfy validation"
    );
    let pk = db
        .table(tid)
        .unwrap()
        .field_by_name("PrimaryKey")
        .unwrap()
        .id;
    let b = db.new_record(tid).unwrap();
    let ti = db.tables.iter().position(|t| t.id == tid).unwrap();
    let key = db.tables[ti].records[a].values[&pk].clone();
    db.tables[ti].records[b].values.insert(pk, key);
    let errs = db.validate(tid, b);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].rule, engine::Rule::Unique);
    assert!(!errs[0].can_override);
    db.tables[ti].records[b].values.insert(pk, String::new());
    assert_eq!(db.validate(tid, b)[0].rule, engine::Rule::NotEmpty);
}

/// Every template's existing records pass their range and calculation
/// validations (FileMaker accepted them).
#[test]
fn template_records_validate() {
    let Some(d) = dir() else { return };
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
            // Auto-entered stamps pass "strict data type: 4-digit year".
            let mut new = db.clone();
            let r = new.new_record(tbl.id).unwrap();
            let strict: Vec<_> = new
                .validate(tbl.id, r)
                .into_iter()
                .filter(|e| matches!(e.rule, engine::Rule::Date | engine::Rule::Time))
                .collect();
            assert!(
                strict.is_empty(),
                "{t}::{} new record: {strict:?}",
                tbl.name
            );
            for r in 0..tbl.records.len() {
                let bad: Vec<_> = db
                    .validate(tbl.id, r)
                    .into_iter()
                    .filter(|e| matches!(e.rule, engine::Rule::Range | engine::Rule::Calculation))
                    .collect();
                assert!(bad.is_empty(), "{t}::{} record {r}: {bad:?}", tbl.name);
            }
        }
    }
}

#[test]
fn export_import_round_trip_and_summaries() {
    use engine::export::{Export, Format, columns};
    use engine::import::{Mapping, Mode, Options, import, read_rows};
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Assets").join("en.fmp12")).unwrap();
    let ti = db.tables.iter().position(|t| t.name == "Assets").unwrap();
    let tid = db.tables[ti].id;
    let to = db
        .occurrences
        .iter()
        .find(|o| o.base_table == tid)
        .unwrap()
        .id;
    let f = |db: &Database, n: &str| db.tables[ti].field_by_name(n).unwrap().id;
    let (name, price, total) = (f(&db, "Name"), f(&db, "Price"), f(&db, "Total"));
    for (n, p) in [
        ("Laptop", "1200"),
        ("Desk", "300.5"),
        ("Chair, \"red\"", ""),
    ] {
        let r = db.new_record(tid).unwrap();
        db.tables[ti].records[r].values.insert(name, n.into());
        if !p.is_empty() {
            db.tables[ti].records[r].values.insert(price, p.into());
        }
    }
    let all: Vec<usize> = (0..3).collect();
    assert_eq!(
        engine::summarize(&db, tid, total, &all, &all).to_text(),
        "1500.5"
    );
    assert_eq!(db.value(tid, 0, total).to_text(), "1500.5");

    let cols = columns(&db, to, &["Name".into(), "Price".into(), "Total".into()]).unwrap();
    for format in [Format::Csv, Format::Tab, Format::Merge, Format::Xlsx] {
        let bytes = Export {
            db: &db,
            to,
            found: &all,
            columns: &cols,
            formatted: false,
            file_name: "Assets.fmp12",
            group_by: &[],
        }
        .write(format);
        let mut copy = db.clone();
        copy.tables[ti].records.clear();
        let header = matches!(format, Format::Merge | Format::Xlsx);
        let o = Options {
            format,
            first_row_names: header,
            mapping: if header {
                Mapping::ByName
            } else {
                Mapping::ByOrder(vec!["Name".into(), "Price".into(), String::new()])
            },
            mode: Mode::Add,
            auto_enter: true,
        };
        let r = import(&mut copy, tid, &[], read_rows(format, &bytes).unwrap(), &o).unwrap();
        assert_eq!(r.added.len(), 3, "{format:?}");
        assert!(r.errors.is_empty(), "{format:?}: {:?}", r.errors);
        for i in 0..3 {
            for fid in [name, price] {
                assert_eq!(
                    copy.tables[ti].records[i].values.get(&fid),
                    db.tables[ti].records[i].values.get(&fid),
                    "{format:?}"
                );
            }
        }
    }
    let xml = Export {
        db: &db,
        to,
        found: &all,
        columns: &cols,
        formatted: true,
        file_name: "Assets.fmp12",
        group_by: &[],
    }
    .write(Format::Xml);
    let xml = String::from_utf8(xml).unwrap();
    assert!(xml.contains("NAME=\"Total\" TYPE=\"NUMBER\""));
    assert!(xml.contains("<DATA>1500.5</DATA>"));
}

#[test]
fn get_summary_by_break_field() {
    let Some(d) = dir() else { return };
    let mut db = Database::open(d.join("Assets").join("en.fmp12")).unwrap();
    let tid = db.tables.iter().find(|t| t.name == "Assets").unwrap().id;
    let ti = db.tables.iter().position(|t| t.id == tid).unwrap();
    let (ty, price) = {
        let t = &db.tables[ti];
        (
            t.field_by_name("Type").unwrap().id,
            t.field_by_name("Price").unwrap().id,
        )
    };
    for (k, p) in [("Laptop", "1000"), ("Phone", "300"), ("Laptop", "500")] {
        let r = db.new_record(tid).unwrap();
        db.tables[ti].records[r].values.insert(ty, k.into());
        db.tables[ti].records[r].values.insert(price, p.into());
    }
    let to = db
        .occurrences
        .iter()
        .find(|o| o.name == "Assets")
        .unwrap()
        .id;
    let ev = |s: &str, r| db.eval_source(s, tid, to, Some(r)).to_text();
    assert_eq!(ev("GetSummary ( Total ; Total )", 0), "1800");
    // Unsorted: no sub-summary.
    assert_eq!(ev("GetSummary ( Total ; Type )", 0), "");
    let mut found: Vec<usize> = (0..3).collect();
    engine::sort(&db, tid, &mut found, &[(ty, Order::Ascending)]);
    let scope = engine::Scope {
        found: Some(engine::FoundSet {
            table: tid,
            records: &found,
            sort: &[ty],
        }),
        self_value: None,
    };
    let ev = |s: &str, r| {
        db.eval_source_in(s, tid, to, Some(r), &scope)
            .unwrap()
            .to_text()
    };
    assert_eq!(ev("GetSummary ( Total ; Type )", 0), "1500");
    assert_eq!(ev("GetSummary ( Total ; Type )", 1), "300");
}
