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

#[test]
fn graph_matches_xml() {
    let mut checked = 0;
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let tos = fmp12::graph::table_occurrences(&file).unwrap();
        let rels = fmp12::graph::relationships(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        let xid = |n: roxmltree::Node| n.attribute("id").unwrap().parse::<u32>().unwrap() & 0xFFF;
        for x in doc
            .descendants()
            .filter(|n| n.has_tag_name("TableOccurrence"))
        {
            let to = tos.iter().find(|t| t.id == xid(x)).expect("TO");
            assert_eq!(to.name, x.attribute("name").unwrap());
            let bt = x
                .descendants()
                .find(|n| n.has_tag_name("BaseTableReference"))
                .unwrap();
            assert_eq!(to.base_table.to_string(), bt.attribute("id").unwrap());
            let r = x.children().find(|n| n.has_tag_name("CoordRect")).unwrap();
            let a = |k: &str| r.attribute(k).unwrap().parse::<f32>().unwrap();
            assert_eq!((to.top, to.left), (a("top"), a("left")), "{}", to.name);
            assert_eq!(
                (to.top + to.height, to.left + to.width),
                (a("bottom"), a("right")),
                "{}",
                to.name
            );
            assert_eq!(
                to.collapsed_height.to_string(),
                x.attribute("height").unwrap()
            );
            let c = x.children().find(|n| n.has_tag_name("Color")).unwrap();
            assert_eq!(to.rgba[0].to_string(), c.attribute("red").unwrap());
            checked += 1;
        }
        for x in doc.descendants().filter(|n| n.has_tag_name("Relationship")) {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let r = rels.iter().find(|r| r.id == id).expect("relationship");
            let side = |tag: &str| x.children().find(|n| n.has_tag_name(tag)).unwrap();
            let (l, rt) = (side("LeftTable"), side("RightTable"));
            let to_of = |n: roxmltree::Node| {
                xid(n
                    .children()
                    .find(|c| c.has_tag_name("TableOccurrenceReference"))
                    .unwrap())
            };
            assert_eq!((r.left, r.right), (to_of(l), to_of(rt)));
            assert_eq!(
                r.left_cascade_create().to_string(),
                l.attribute("cascadeCreate").unwrap().to_lowercase()
            );
            assert_eq!(
                r.right_cascade_create().to_string(),
                rt.attribute("cascadeCreate").unwrap().to_lowercase()
            );
            let preds: Vec<_> = x
                .descendants()
                .filter(|n| n.has_tag_name("JoinPredicate"))
                .collect();
            assert_eq!(r.predicates.len(), preds.len());
            for (p, xp) in r.predicates.iter().zip(&preds) {
                let fid = |tag: &str| {
                    xp.descendants()
                        .find(|n| n.has_tag_name(tag))
                        .unwrap()
                        .first_element_child()
                        .unwrap()
                        .attribute("id")
                        .unwrap()
                        .parse::<u32>()
                        .unwrap()
                };
                assert_eq!(
                    (p.left_field, p.right_field),
                    (fid("LeftField"), fid("RightField"))
                );
            }
            checked += 1;
        }
    }
    eprintln!("{checked} graph objects checked against XML");
}

#[test]
fn scripts_match_xml() {
    let (mut scripts_checked, mut steps_checked) = (0, 0);
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let scripts = fmp12::scripts::scripts(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        let cat = doc
            .descendants()
            .find(|n| n.has_tag_name("ScriptCatalog"))
            .unwrap();
        for x in cat.children().filter(|n| n.has_tag_name("Script")) {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let s = scripts.iter().find(|s| s.id == id).expect("script");
            assert_eq!(s.name, x.attribute("name").unwrap());
            let opts = x.children().find(|n| n.has_tag_name("Options")).unwrap();
            assert_eq!(s.options.to_string(), opts.text().unwrap(), "{}", s.name);
            scripts_checked += 1;
        }
        for x in doc
            .descendants()
            .filter(|n| n.has_tag_name("StepsForScripts"))
            .flat_map(|n| n.children())
        {
            let Some(r) = x.children().find(|n| n.has_tag_name("ScriptReference")) else {
                continue;
            };
            let id: u32 = r.attribute("id").unwrap().parse().unwrap();
            let s = scripts.iter().find(|s| s.id == id).unwrap();
            let steps: Vec<_> = x.descendants().filter(|n| n.has_tag_name("Step")).collect();
            assert_eq!(s.steps.len(), steps.len(), "{}", s.name);
            for (st, xs) in s.steps.iter().zip(&steps) {
                assert_eq!(st.id.to_string(), xs.attribute("id").unwrap(), "{}", s.name);
                assert_eq!(
                    st.enabled.to_string(),
                    xs.attribute("enable").unwrap().to_lowercase()
                );
                let o = xs.children().find(|n| n.has_tag_name("Options")).unwrap();
                assert_eq!(
                    st.options.to_string(),
                    o.text().unwrap(),
                    "{} step {}",
                    s.name,
                    st.id
                );
                steps_checked += 1;
            }
        }
    }
    eprintln!("{scripts_checked} scripts, {steps_checked} steps checked against XML");
}
