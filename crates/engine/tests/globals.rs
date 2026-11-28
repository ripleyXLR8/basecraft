//! Global fields, multi-key relationships and second-field value lists, on
//! files built here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use engine::Database;
use fmp12::graph::{JoinOp, Predicate};
use fmp12::schema::DataType;
use fmp12::schema_edit::{FieldSpec, RelationshipSpec};

fn global(name: &str) -> FieldSpec {
    let mut f = FieldSpec::new(name, DataType::Text);
    f.storage.global = true;
    f
}

#[test]
fn a_global_has_one_value_per_table_and_is_saved() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Notes").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.tables[0].id;
    let g = db.add_field(&mut file, t, &global("gFilter")).unwrap();
    let name = db
        .add_field(&mut file, t, &FieldSpec::new("Name", DataType::Text))
        .unwrap();
    // Set with no record at all, then seen from every record.
    db.set_values(t, 0, &[(g, "yes".into())]);
    assert_eq!(db.value(t, 0, g).to_text(), "yes");
    let a = db.new_record(t).unwrap();
    let b = db.new_record(t).unwrap();
    db.set_values(t, a, &[(name, "Ada".into())]);
    db.set_values(t, b, &[(g, "no".into())]);
    assert_eq!(db.value(t, a, g).to_text(), "no");
    assert!(!db.tables[0].records[b].values.contains_key(&g));

    // Saved at [T].[8], read back from there.
    let file = fmp12::File::from_bytes(db.save(&file).unwrap()).unwrap();
    let table = fmp12::schema::tables(&file).unwrap().remove(0);
    assert_eq!(fmp12::schema::globals(&file, &table).unwrap()[&g], "no");
    let db = Database::load(&file).unwrap();
    assert_eq!(db.value(t, 1, g).to_text(), "no");
    assert_eq!(db.value(t, 0, name).to_text(), "Ada");
}

/// FileMaker's "type-ahead" relationship: a key of several lines matches
/// when one of its lines satisfies each predicate.
#[test]
fn multi_line_keys_match_line_by_line() {
    let mut file =
        fmp12::File::from_bytes(fmp12::schema_edit::new_file("Search").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let s = db.tables[0].id;
    let range = db
        .add_field(
            &mut file,
            s,
            &FieldSpec::calculated("Range", DataType::Text, "\"B\" & ¶ & \"D\""),
        )
        .unwrap();
    let p = db.add_table(&mut file, "People").unwrap();
    let name = db
        .add_field(&mut file, p, &FieldSpec::new("Name", DataType::Text))
        .unwrap();
    let to = |db: &Database, n: &str| db.occurrences.iter().find(|o| o.name == n).unwrap().id;
    let (sto, pto) = (to(&db, "Search"), to(&db, "People"));
    let mut rel = RelationshipSpec::equal(sto, range, pto, name);
    rel.predicates = vec![
        Predicate {
            op: JoinOp::GreaterOrEqual,
            left_field: range,
            right_field: name,
        },
        Predicate {
            op: JoinOp::LessOrEqual,
            left_field: range,
            right_field: name,
        },
    ];
    db.add_relationship(&mut file, &rel).unwrap();
    db.new_record(s).unwrap();
    for n in ["Ada", "Bob", "Cat", "Dan", "C\rZed"] {
        let r = db.new_record(p).unwrap();
        db.set_values(p, r, &[(name, n.into())]);
    }
    let names: Vec<String> = db
        .related(sto, 0, pto)
        .iter()
        .map(|&r| db.value(p, r, name).to_text())
        .collect();
    // "B" ≤ name ≤ "D"; a right-hand value of several lines is several keys.
    assert_eq!(names, ["Bob", "Cat", "C\rZed"]);
}

#[test]
fn second_field_labels() {
    let mut file =
        fmp12::File::from_bytes(fmp12::schema_edit::new_file("Events").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.tables[0].id;
    let id = db
        .add_field(&mut file, t, &FieldSpec::new("Id", DataType::Number))
        .unwrap();
    let title = db
        .add_field(&mut file, t, &FieldSpec::new("Title", DataType::Text))
        .unwrap();
    let to = db.occurrences[0].id;
    let source = |second_only| fmp12::valuelists::Source::Field {
        to,
        field: id,
        second: Some((to, title)),
        second_only,
        sort_by_second: false,
        related_from: None,
    };
    let only = db.add_value_list(&mut file, "Only", &source(true)).unwrap();
    let both = db
        .add_value_list(&mut file, "Both", &source(false))
        .unwrap();
    for (i, n) in [("1", "Rome"), ("2", "Oslo")] {
        let r = db.new_record(t).unwrap();
        db.set_values(t, r, &[(id, i.into()), (title, n.into())]);
    }
    let labels = db.value_list_labels(only).unwrap();
    assert_eq!(labels["1"], "Rome");
    assert_eq!(labels["2"], "Oslo");
    assert!(db.value_list_labels(both).is_none());
}
