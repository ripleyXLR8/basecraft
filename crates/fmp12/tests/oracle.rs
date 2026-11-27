//! Compares decoded schema with FileMaker's own "Save a Copy as XML" export.
//!
//! Needs a directory (env `FMP12_ORACLE`) holding pairs `<name>.fmp12` +
//! `<name>.xml`, produced locally with `tools/oracle`. Skipped when unset:
//! those files derive from Claris templates and are not committed.

use fmp12::schema::{self, AutoStamp, DataType, FieldKind, Indexing};
use std::path::PathBuf;

fn pairs() -> Vec<(PathBuf, PathBuf)> {
    let Some(dir) = std::env::var_os("FMP12_ORACLE") else {
        eprintln!("FMP12_ORACLE not set, skipping");
        return vec![];
    };
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap() {
        let x = e.unwrap().path();
        let f = x.with_extension("fmp12");
        if x.extension().is_some_and(|e| e == "xml") && f.exists() {
            out.push((f, x));
        }
    }
    out
}

fn stamp_name(a: AutoStamp) -> &'static str {
    use AutoStamp::*;
    match a {
        CreationDate => "CreationDate",
        CreationTime => "CreationTime",
        CreationTimestamp => "CreationTimestamp",
        CreationName => "CreationName",
        CreationAccountName => "CreationAccountName",
        ModificationDate => "ModificationDate",
        ModificationTime => "ModificationTime",
        ModificationTimestamp => "ModificationTimestamp",
        ModificationName => "ModificationName",
        ModificationAccountName => "ModificationAccountName",
    }
}

#[test]
fn fields_match_xml() {
    let mut checked = 0;
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let tables = schema::tables(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        for cat in doc.descendants().filter(|n| n.has_tag_name("FieldCatalog")) {
            let tid: u32 = cat
                .children()
                .find(|n| n.has_tag_name("BaseTableReference"))
                .unwrap()
                .attribute("id")
                .unwrap()
                .parse()
                .unwrap();
            let table = tables.iter().find(|t| t.id == tid).expect("table");
            for x in cat
                .descendants()
                .filter(|n| n.has_tag_name("Field") && n.attribute("datatype").is_some())
            {
                let id: u32 = x.attribute("id").unwrap().parse().unwrap();
                let f = &table.fields[&id];
                let ctx = format!("{} {}::{}", xml.display(), table.name, f.name);
                let child = |tag: &str| x.children().find(|n| n.has_tag_name(tag));
                assert_eq!(f.name, x.attribute("name").unwrap(), "{ctx}");
                assert_eq!(f.comment, x.attribute("comment").unwrap_or(""), "{ctx}");
                let kind = match f.kind() {
                    FieldKind::Normal => "Normal",
                    FieldKind::Calculated => "Calculated",
                    FieldKind::Summary => "Summary",
                    FieldKind::Unknown(_) => "?",
                };
                assert_eq!(kind, x.attribute("fieldtype").unwrap(), "{ctx}");
                let dt = match f.data_type() {
                    DataType::Text => "Text",
                    DataType::Number => "Number",
                    DataType::Date => "Date",
                    DataType::Time => "Time",
                    DataType::Timestamp => "Timestamp",
                    DataType::Container => "Binary",
                    DataType::Unknown(_) => "?",
                };
                assert_eq!(dt, x.attribute("datatype").unwrap(), "{ctx}");
                if let Some(st) = child("Storage") {
                    assert_eq!(
                        f.global().to_string(),
                        st.attribute("global").unwrap().to_lowercase(),
                        "{ctx}"
                    );
                    let reps: u16 = st.attribute("maxRepetitions").unwrap().parse().unwrap();
                    assert_eq!(f.max_repetitions(), reps, "{ctx}");
                    if let Some(ix) = st.attribute("index") {
                        let want = match f.indexing() {
                            Indexing::None => "None",
                            Indexing::Minimal => "Minimal",
                            Indexing::All => "All",
                        };
                        assert_eq!(want, ix, "{ctx}");
                    }
                }
                if let Some(ae) = child("AutoEnter") {
                    let ty = ae.attribute("type").unwrap_or("");
                    let ours = if let Some(s) = f.auto_stamp() {
                        stamp_name(s)
                    } else if f.auto_calculation() {
                        "Calculated"
                    } else if f.auto_serial() {
                        "SerialNumber"
                    } else if f.auto_lookup() {
                        "Looked_up"
                    } else if f.auto_constant() {
                        "ConstantData"
                    } else if f.auto_last_visited() {
                        "LastVisited"
                    } else {
                        ""
                    };
                    assert_eq!(ours, ty, "{ctx}");
                    if let Some(p) = ae.attribute("prohibitModification") {
                        assert_eq!(
                            f.prohibit_modification().to_string(),
                            p.to_lowercase(),
                            "{ctx}"
                        );
                    }
                }
                if let Some(v) = child("Validation") {
                    let ours = f.validation();
                    let b = |a: &str| v.attribute(a).map(|s| s == "True");
                    assert_eq!(Some(ours.not_empty), b("notEmpty"), "{ctx}");
                    assert_eq!(Some(ours.unique), b("unique"), "{ctx}");
                    assert_eq!(Some(ours.existing), b("existing"), "{ctx}");
                    assert_eq!(Some(ours.allow_override), b("allowOverride"), "{ctx}");
                    assert_eq!(
                        Some(ours.always),
                        v.attribute("type").map(|t| t == "Always"),
                        "{ctx}"
                    );
                }
                checked += 1;
            }
        }
    }
    eprintln!("{checked} fields checked against XML");
}
