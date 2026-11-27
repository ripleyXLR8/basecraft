//! Schema editing through the engine, then records written with `save`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use engine::Database;
use fmp12::schema::DataType;
use fmp12::schema_edit::FieldSpec;

#[test]
fn new_file_new_table_records_saved() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Notes").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.add_table(&mut file, "People").unwrap();
    let mut id = FieldSpec::new("Id", DataType::Number);
    id.auto_enter.serial = Some("1".into());
    let fid = db.add_field(&mut file, t, &id).unwrap();
    let name = db
        .add_field(&mut file, t, &FieldSpec::new("Name", DataType::Text))
        .unwrap();
    let label = db
        .add_field(
            &mut file,
            t,
            &FieldSpec::calculated("Label", DataType::Text, "Name & \" #\" & Id"),
        )
        .unwrap();
    // A record created before a later schema change survives it.
    let r = db.new_record(t).unwrap();
    db.tables.iter_mut().find(|x| x.id == t).unwrap().records[r]
        .values
        .insert(name, "Ada".into());
    db.add_field(&mut file, t, &FieldSpec::new("Born", DataType::Date))
        .unwrap();
    assert_eq!(db.value(t, r, label).to_text(), "Ada #1");
    let bytes = db.save(&file).unwrap();
    let file = fmp12::File::from_bytes(bytes).unwrap();
    let db = Database::load(&file).unwrap();
    let p = db.table(t).unwrap();
    assert_eq!(p.records.len(), 1);
    assert_eq!(p.records[0].values[&fid], "1");
    // Stored calculation written on save.
    assert_eq!(p.records[0].values[&label], "Ada #1");
}

#[test]
fn rename_and_delete_through_the_engine() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Notes").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.tables[0].id;
    let f = db
        .add_field(&mut file, t, &FieldSpec::new("Text", DataType::Text))
        .unwrap();
    let r = db.new_record(t).unwrap();
    db.tables[0].records[r].values.insert(f, "x".into());
    db.rename_table(&mut file, t, "Memos").unwrap();
    db.rename_field(&mut file, t, f, "Body").unwrap();
    assert_eq!(db.tables[0].name, "Memos");
    assert_eq!(db.tables[0].fields[0].name, "Body");
    assert_eq!(db.tables[0].records[0].values[&f], "x");
    db.delete_field(&mut file, t, f).unwrap();
    assert!(db.tables[0].records[0].values.is_empty());
    let t2 = db.add_table(&mut file, "Other").unwrap();
    db.delete_table(&mut file, t2).unwrap();
    assert_eq!(db.tables.len(), 1);
}

#[test]
fn saving_moves_the_next_serial_value() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("T").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.tables[0].id;
    let mut id = FieldSpec::new("Id", DataType::Number);
    id.auto_enter.serial = Some("1".into());
    let f = db.add_field(&mut file, t, &id).unwrap();
    db.new_record(t).unwrap();
    db.new_record(t).unwrap();
    let file = fmp12::File::from_bytes(db.save(&file).unwrap()).unwrap();
    let ed = fmp12::schema_edit::Editor::open(&file).unwrap();
    assert_eq!(ed.next_serial(t, f).as_deref(), Some("3"));
}
