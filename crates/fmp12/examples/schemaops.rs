//! Applies schema operations to a file, for experiments with FileMaker.
//! `cargo run -p fmp12 --example schemaops -- <in|new> <out> op…`
//! ops: `table:Name`, `field:Table:Name:text|number|date|time|timestamp|container`,
//! `calc:Table:Name:formula`, `rename-table:Old:New`, `delete-table:Name`,
//! `rename-field:Table:Old:New`, `delete-field:Table:Name`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::schema::DataType;
use fmp12::schema_edit::{Editor, FieldSpec};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let bytes = if a[0] == "new" {
        fmp12::schema_edit::new_file("Blank").unwrap()
    } else {
        std::fs::read(&a[0]).unwrap()
    };
    let file = fmp12::File::from_bytes(bytes).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    for op in &a[2..] {
        let p: Vec<&str> = op.splitn(4, ':').collect();
        let t = |n: &str| ed.table_id(n).unwrap();
        match p[0] {
            "table" => eprintln!("{:?}", ed.add_table(p[1]).unwrap()),
            "field" => {
                let dt = match p[3] {
                    "number" => DataType::Number,
                    "date" => DataType::Date,
                    "time" => DataType::Time,
                    "timestamp" => DataType::Timestamp,
                    "container" => DataType::Container,
                    _ => DataType::Text,
                };
                let tid = t(p[1]);
                eprintln!("{}", ed.add_field(tid, &FieldSpec::new(p[2], dt)).unwrap());
            }
            "calc" => {
                let tid = t(p[1]);
                let spec = FieldSpec::calculated(p[2], DataType::Text, p[3]);
                eprintln!("{}", ed.add_field(tid, &spec).unwrap());
            }
            "rename-table" => {
                let tid = t(p[1]);
                ed.rename_table(tid, p[2]).unwrap()
            }
            "delete-table" => {
                let tid = t(p[1]);
                ed.delete_table(tid).unwrap()
            }
            "rename-field" => {
                let tid = t(p[1]);
                let f = ed.field_id(tid, p[2]).unwrap();
                ed.rename_field(tid, f, p[3]).unwrap()
            }
            "delete-field" => {
                let tid = t(p[1]);
                let f = ed.field_id(tid, p[2]).unwrap();
                ed.delete_field(tid, f).unwrap()
            }
            _ => panic!("bad op {op}"),
        }
    }
    std::fs::write(&a[1], ed.finish()).unwrap();
}
