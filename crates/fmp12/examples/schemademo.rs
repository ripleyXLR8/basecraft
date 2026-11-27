//! Builds a file with a new table holding one field of each kind, for
//! checking schema writing in FileMaker (`tools/oracle/README.md`).
//! `cargo run -p fmp12 --example schemademo -- <out.fmp12> [base.fmp12]`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::schema::{AutoStamp, DataType, Indexing, SummaryOp};
use fmp12::schema_edit::{Editor, FieldSpec};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let base = match a.get(1) {
        Some(p) => std::fs::read(p).unwrap(),
        None => fmp12::schema_edit::new_file("Contacts").unwrap(),
    };
    let file = fmp12::File::from_bytes(base).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let (t, to, layout) = ed.add_table("People").unwrap();
    eprintln!("table {t}, occurrence {to}, layout {layout:?}");
    let mut id = FieldSpec::new("Id", DataType::Number);
    id.auto_enter.serial = Some("1".into());
    id.validation.unique = true;
    ed.add_field(t, &id).unwrap();
    let mut name = FieldSpec::new("Name", DataType::Text);
    name.comment = "Full name".into();
    ed.add_field(t, &name).unwrap();
    let mut age = FieldSpec::new("Age", DataType::Number);
    age.validation.range = Some(("0".into(), "150".into()));
    age.validation.calculation = Some("Age >= 0".into());
    ed.add_field(t, &age).unwrap();
    let mut created = FieldSpec::new("Created", DataType::Timestamp);
    created.auto_enter.stamp = Some(AutoStamp::CreationTimestamp);
    ed.add_field(t, &created).unwrap();
    let mut modified = FieldSpec::new("Modified", DataType::Timestamp);
    modified.auto_enter.stamp = Some(AutoStamp::ModificationTimestamp);
    ed.add_field(t, &modified).unwrap();
    let mut country = FieldSpec::new("Country", DataType::Text);
    country.auto_enter.constant = Some("France".into());
    country.storage.indexing = Indexing::All;
    ed.add_field(t, &country).unwrap();
    let mut key = FieldSpec::new("Key", DataType::Text);
    key.auto_enter.calculation = Some("Get ( UUID )".into());
    ed.add_field(t, &key).unwrap();
    let mut code = FieldSpec::new("Code", DataType::Text);
    code.auto_enter.calculation = Some("Upper ( Left ( Country ; 2 ) ) & \"-\" & Id".into());
    ed.add_field(t, &code).unwrap();
    let mut g = FieldSpec::new("Global note", DataType::Text);
    g.storage.global = true;
    ed.add_field(t, &g).unwrap();
    let mut reps = FieldSpec::new("Phones", DataType::Text);
    reps.storage.repetitions = 3;
    ed.add_field(t, &reps).unwrap();
    ed.add_field(t, &FieldSpec::new("Photo", DataType::Container))
        .unwrap();
    ed.add_field(t, &FieldSpec::new("Born", DataType::Date))
        .unwrap();
    ed.add_field(t, &FieldSpec::new("Alarm", DataType::Time))
        .unwrap();
    ed.add_field(
        t,
        &FieldSpec::calculated(
            "Label",
            DataType::Text,
            "Country & \" #\" & Id & \" \" & Case ( Id > 1 ; \"many\" ; \"first\" )",
        ),
    )
    .unwrap();
    let mut double = FieldSpec::calculated("Double", DataType::Number, "Id * 2");
    double.field_type = fmp12::schema_edit::FieldType::Calculated {
        formula: "Id * 2".into(),
        stored: false,
    };
    ed.add_field(t, &double).unwrap();
    let id_f = ed.field_id(t, "Id").unwrap();
    ed.add_field(t, &FieldSpec::summary("Total ids", SummaryOp::Total, id_f))
        .unwrap();
    std::fs::write(&a[0], ed.finish()).unwrap();
}
