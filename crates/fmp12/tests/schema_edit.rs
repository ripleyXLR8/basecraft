//! Schema editing: tables, fields and options written by `schema_edit`
//! read back through the reader (and, where present, the templates).

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::schema::{AutoStamp, DataType, FieldKind, Indexing, SummaryOp};
use fmp12::schema_edit::{Editor, FieldSpec, FieldType};

fn reopen(bytes: Vec<u8>) -> fmp12::File {
    let f = fmp12::File::from_bytes(bytes).unwrap();
    // Every leaf decodes and the entries are in store order.
    let mut es = fmp12::write::entries(&f).unwrap();
    fmp12::write::drop_leftovers(&mut es);
    for w in es.windows(2) {
        assert!(fmp12::write::sort_key(&w[0]) <= fmp12::write::sort_key(&w[1]));
    }
    f
}

/// A table with one field of each kind and option, built on a new file.
fn people() -> (fmp12::File, u32) {
    let file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Contacts").unwrap()).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let (t, _, layout) = ed.add_table("People").unwrap();
    assert!(layout.is_some());
    let mut id = FieldSpec::new("Id", DataType::Number);
    id.auto_enter.serial = Some("1".into());
    id.validation.unique = true;
    ed.add_field(t, &id).unwrap();
    let mut name = FieldSpec::new("Name", DataType::Text);
    name.validation.not_empty = true;
    name.comment = "Full name".into();
    ed.add_field(t, &name).unwrap();
    let mut age = FieldSpec::new("Age", DataType::Number);
    age.validation.range = Some(("0".into(), "150".into()));
    age.validation.calculation = Some("Age >= 0".into());
    ed.add_field(t, &age).unwrap();
    let mut created = FieldSpec::new("Created", DataType::Timestamp);
    created.auto_enter.stamp = Some(AutoStamp::CreationTimestamp);
    ed.add_field(t, &created).unwrap();
    let mut country = FieldSpec::new("Country", DataType::Text);
    country.auto_enter.constant = Some("France".into());
    country.storage.indexing = Indexing::All;
    ed.add_field(t, &country).unwrap();
    let mut key = FieldSpec::new("Key", DataType::Text);
    key.auto_enter.calculation = Some("Get ( UUID )".into());
    ed.add_field(t, &key).unwrap();
    let mut g = FieldSpec::new("Global note", DataType::Text);
    g.storage.global = true;
    ed.add_field(t, &g).unwrap();
    let mut reps = FieldSpec::new("Phones", DataType::Text);
    reps.storage.repetitions = 3;
    ed.add_field(t, &reps).unwrap();
    ed.add_field(t, &FieldSpec::new("Photo", DataType::Container))
        .unwrap();
    ed.add_field(
        t,
        &FieldSpec::calculated("Label", DataType::Text, "Name & \" (\" & Age & \")\""),
    )
    .unwrap();
    let age_id = ed.field_id(t, "Age").unwrap();
    ed.add_field(
        t,
        &FieldSpec::summary("Total age", SummaryOp::Total, age_id),
    )
    .unwrap();
    (reopen(ed.finish()), t)
}

#[test]
fn new_file_has_one_empty_table() {
    let f = reopen(fmp12::schema_edit::new_file("Contacts").unwrap());
    let tables = fmp12::schema::tables(&f).unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "Contacts");
    let tos = fmp12::graph::table_occurrences(&f).unwrap();
    assert_eq!(tos[0].name, "Contacts");
    let layouts = fmp12::layouts::layouts(&f).unwrap();
    assert!(layouts.iter().any(|l| l.name == "Contacts"));
}

#[test]
fn added_table_and_fields_read_back() {
    let (f, t) = people();
    let tables = fmp12::schema::tables(&f).unwrap();
    let p = tables.iter().find(|x| x.id == t).unwrap();
    assert_eq!(p.name, "People");
    let by = |n: &str| p.fields.values().find(|x| x.name == n).unwrap();
    assert!(by("Id").auto_serial());
    assert!(by("Id").validation().unique);
    assert_eq!(by("Id").indexing(), Indexing::Minimal);
    assert!(by("Name").validation().not_empty);
    assert_eq!(by("Name").comment, "Full name");
    assert!(by("Age").validation().range && by("Age").validation().calculation);
    assert_eq!(
        by("Created").auto_stamp(),
        Some(AutoStamp::CreationTimestamp)
    );
    assert!(by("Country").auto_constant());
    assert_eq!(by("Country").indexing(), Indexing::All);
    assert!(by("Key").auto_calculation());
    assert!(by("Global note").global());
    assert_eq!(by("Phones").max_repetitions(), 3);
    assert_eq!(by("Photo").data_type(), DataType::Container);
    assert_eq!(by("Label").kind(), FieldKind::Calculated);
    let total = by("Total age").summary().unwrap();
    assert_eq!(total.op, SummaryOp::Total);
    assert_eq!(p.fields[&total.field].name, "Age");
    // Occurrence and layout of the table.
    let to = fmp12::graph::table_occurrences(&f)
        .unwrap()
        .into_iter()
        .find(|o| o.base_table == t)
        .unwrap();
    assert_eq!(to.name, "People");
    let layouts = fmp12::layouts::layouts(&f).unwrap();
    let l = layouts.iter().find(|l| l.name == "People").unwrap();
    assert_eq!(l.table_occurrence & 0xFFF, to.id as u64);
    // Calculations decompile to their source, with names resolved.
    let ed = Editor::open(&f).unwrap();
    let label = ed.field_spec(t, ed.field_id(t, "Label").unwrap()).unwrap();
    assert_eq!(
        label.field_type,
        FieldType::Calculated {
            formula: "Name & \" (\" & Age & \")\"".into(),
            stored: true
        }
    );
    let age = ed.field_spec(t, ed.field_id(t, "Age").unwrap()).unwrap();
    assert_eq!(age.validation.calculation.as_deref(), Some("Age >= 0"));
    assert_eq!(age.validation.range, Some(("0".into(), "150".into())));
    let key = ed.field_spec(t, ed.field_id(t, "Key").unwrap()).unwrap();
    assert_eq!(key.auto_enter.calculation.as_deref(), Some("Get ( UUID )"));
}

#[test]
fn rename_and_delete() {
    let (f, t) = people();
    let mut ed = Editor::open(&f).unwrap();
    ed.rename_table(t, "Persons").unwrap();
    let name = ed.field_id(t, "Name").unwrap();
    ed.rename_field(t, name, "Full name").unwrap();
    assert!(
        ed.add_field(t, &FieldSpec::new("full NAME", DataType::Text))
            .is_err()
    );
    let photo = ed.field_id(t, "Photo").unwrap();
    ed.delete_field(t, photo).unwrap();
    // The calculation follows the rename (it stores field ids).
    let label = ed.field_spec(t, ed.field_id(t, "Label").unwrap()).unwrap();
    assert!(
        matches!(label.field_type, FieldType::Calculated { ref formula, .. } if formula.starts_with("Full name &"))
    );
    let f = reopen(ed.finish());
    let tables = fmp12::schema::tables(&f).unwrap();
    let p = tables.iter().find(|x| x.id == t).unwrap();
    assert_eq!(p.name, "Persons");
    assert!(p.fields.values().any(|x| x.name == "Full name"));
    assert!(!p.fields.values().any(|x| x.name == "Photo"));
    let tos = fmp12::graph::table_occurrences(&f).unwrap();
    assert!(tos.iter().any(|o| o.name == "Persons"));
    assert!(
        fmp12::layouts::layouts(&f)
            .unwrap()
            .iter()
            .any(|l| l.name == "Persons")
    );
    let mut ed = Editor::open(&f).unwrap();
    ed.delete_table(t).unwrap();
    let f = reopen(ed.finish());
    assert!(!fmp12::schema::tables(&f).unwrap().iter().any(|x| x.id == t));
    assert!(
        !fmp12::graph::table_occurrences(&f)
            .unwrap()
            .iter()
            .any(|o| o.base_table == t)
    );
}

#[test]
fn records_in_a_new_table() {
    let (f, t) = people();
    let tables = fmp12::schema::tables(&f).unwrap();
    let p = tables.iter().find(|x| x.id == t).unwrap();
    let id = |n: &str| *p.fields.iter().find(|(_, x)| x.name == n).unwrap().0;
    let mut recs = fmp12::schema::Records::new();
    recs.insert(
        1,
        [(id("Name"), "Ada".to_string()), (id("Age"), "36".into())].into(),
    );
    let out = fmp12::save::save_records(&f, &[(t, recs.clone())].into()).unwrap();
    let f = reopen(out);
    let back = fmp12::schema::records(&f, p).unwrap();
    assert_eq!(back, recs);
}

#[test]
fn bad_calculation_writes_nothing() {
    let file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("T").unwrap()).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let t = ed.table_id("T").unwrap();
    let spec = FieldSpec::calculated("C", DataType::Number, "Nope + 1");
    assert!(ed.add_field(t, &spec).is_err());
    assert_eq!(ed.field_id(t, "C"), None);
}
