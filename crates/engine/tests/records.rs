//! Typed record values and saving, on files where we typed records in
//! FileMaker Pro 26. They are copies of Claris templates, so they stay
//! local (`crates/fmp12/tests/fixtures/local`); tests skip without them.

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use calc::Value;
use calc::value::{day_number, parse_time};
use engine::{Database, typed_in};
use fmp12::schema::{DataType, DateOrder, Locale};
use std::path::PathBuf;

fn local(name: &str) -> Option<(fmp12::File, Database)> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fmp12/tests/fixtures/local")
        .join(name);
    if !p.is_file() {
        eprintln!("no {name}, skipping");
        return None;
    }
    let f = fmp12::File::open(p).unwrap();
    let db = Database::load(&f).unwrap();
    Some((f, db))
}

fn value(db: &Database, table: &str, record: u32, field: &str) -> Value {
    let t = db.tables.iter().find(|t| t.name == table).unwrap();
    let i = t.records.iter().position(|r| r.id == record).unwrap();
    db.value(t.id, i, t.field_by_name(field).unwrap().id)
}

#[test]
fn locale_formats() {
    let fr = Locale {
        date_order: DateOrder::Dmy,
        decimal: ',',
        thousands: '\u{A0}',
        am: String::new(),
        pm: String::new(),
        ..Locale::default()
    };
    let d = typed_in("01/02/2026", DataType::Date, &fr);
    assert_eq!(d, Value::Date(day_number(2026, 2, 1)));
    let n = typed_in("12,75", DataType::Number, &fr).to_text();
    assert_eq!(n, "12.75");
    let ts = typed_in("27/11/2026 14:35:26", DataType::Timestamp, &fr);
    assert!(matches!(ts, Value::Timestamp(_)));
    let en = Locale::default();
    let t = typed_in("2:35:03 PM", DataType::Time, &en);
    assert_eq!(t, Value::Time(parse_time("14:35:03").unwrap()));
}

#[test]
fn typed_values_from_filemaker() {
    if let Some((_, db)) = local("meetings_en.fmp12") {
        let start = value(&db, "Meetings", 1, "Start Time");
        assert_eq!(start, Value::Time(parse_time("14:30").unwrap()));
        let date = value(&db, "Meetings", 1, "Date");
        assert_eq!(date, Value::Date(day_number(2026, 2, 1)));
    }
    if let Some((_, db)) = local("meetings_fr.fmp12") {
        // Typed "01/02/2026" in a French file: 1 February, stored as typed.
        let date = value(&db, "Réunions", 1, "Date");
        assert_eq!(date, Value::Date(day_number(2026, 2, 1)));
        let end = value(&db, "Réunions", 1, "Heure de fin");
        assert_eq!(end, Value::Time(parse_time("9:05:30").unwrap()));
    }
    if let Some((_, db)) = local("inventory_records.fmp12") {
        let year = value(&db, "Products", 2, "Model Year").to_text();
        assert_eq!(year, "1999");
        let created = value(&db, "Products", 2, "CreationTimestamp");
        assert!(matches!(created, Value::Timestamp(_)));
    }
}

/// Add, edit and delete through the engine, save, read back.
#[test]
fn save_roundtrip() {
    let Some((f, mut db)) = local("inventory_records.fmp12") else {
        return;
    };
    let t = db.tables.iter_mut().find(|t| t.name == "Products").unwrap();
    let name = t.field_by_name("Name").unwrap().id;
    let i = t.new_record();
    assert_eq!(
        t.records[i].id, 41,
        "ids continue after the last one ever used"
    );
    t.records[i]
        .values
        .insert(name, "Nouveau — produit é".into());
    t.records[0].values.insert(name, "Renamed ✓".into());
    t.records.retain(|r| r.id != 3);
    let bytes = db.save(&f).unwrap();
    let again = Database::load(&fmp12::File::from_bytes(bytes).unwrap()).unwrap();
    let (a, b) = (&db.tables[0], &again.tables[0]);
    assert_eq!(b.records.len(), 39);
    for (x, y) in a.records.iter().zip(&b.records) {
        assert_eq!(x.id, y.id);
        let nonempty = x.values.iter().filter(|(_, v)| !v.is_empty());
        assert!(nonempty.eq(y.values.iter()), "record {}", x.id);
    }
    // Unchanged tables are untouched.
    assert_eq!(db.tables[1].records.len(), again.tables[1].records.len());
}
