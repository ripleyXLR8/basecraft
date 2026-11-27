//! Compares decoded schema with FileMaker's own "Save a Copy as XML" export.
//!
//! Needs a directory (env `FMP12_ORACLE`) holding pairs `<name>.fmp12` +
//! `<name>.xml`, produced locally with `tools/oracle`. Skipped when unset:
//! those files derive from Claris templates and are not committed.

// Helpers outside #[test] functions may unwrap, expect and panic too.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

#[test]
fn value_lists_match_xml() {
    use fmp12::valuelists::Source;
    let mut checked = 0;
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let vls = fmp12::valuelists::value_lists(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        for x in doc.descendants().filter(|n| n.has_tag_name("ValueList")) {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let v = vls.iter().find(|v| v.id == id).expect("value list");
            assert_eq!(v.name, x.attribute("name").unwrap());
            let src = x
                .children()
                .find(|n| n.has_tag_name("Source"))
                .unwrap()
                .attribute("value")
                .unwrap();
            match &v.source {
                Source::Custom(t) => {
                    assert_eq!(src, "Custom", "{}", v.name);
                    let want = x
                        .descendants()
                        .find(|n| n.has_tag_name("Text"))
                        .and_then(|n| n.text())
                        .unwrap_or("");
                    assert_eq!(
                        t.replace('\r', "\n"),
                        want.replace("\r\n", "\n"),
                        "{}",
                        v.name
                    );
                }
                Source::Field { to, field, .. } => {
                    assert_eq!(src, "FromField", "{}", v.name);
                    let pf = x
                        .descendants()
                        .find(|n| n.has_tag_name("PrimaryField"))
                        .unwrap();
                    let fr = pf.first_element_child().unwrap();
                    assert_eq!(field.to_string(), fr.attribute("id").unwrap(), "{}", v.name);
                    let tor = fr
                        .first_element_child()
                        .unwrap()
                        .attribute("id")
                        .unwrap()
                        .parse::<u32>()
                        .unwrap();
                    assert_eq!(*to, tor & 0xFFF, "{}", v.name);
                }
                Source::Unknown(n) => panic!("{}: unknown source {n} vs {src}", v.name),
            }
            checked += 1;
        }
    }
    eprintln!("{checked} value lists checked against XML");
}

#[test]
fn layouts_match_xml() {
    let (mut layouts_checked, mut objects_checked, mut parents_ok) = (0, 0, 0);
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let layouts = fmp12::layouts::layouts(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        let cat = doc
            .descendants()
            .find(|n| n.has_tag_name("LayoutCatalog"))
            .unwrap();
        for x in cat.descendants().filter(|n| n.has_tag_name("Layout")) {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let l = layouts.iter().find(|l| l.id == id).expect("layout");
            assert_eq!(l.name, x.attribute("name").unwrap());
            if x.attribute("isFolder").is_some() {
                continue;
            }
            assert_eq!(
                l.width.to_string(),
                x.attribute("width").unwrap(),
                "{}",
                l.name
            );
            let to = x
                .children()
                .find(|n| n.has_tag_name("TableOccurrenceReference"))
                .unwrap();
            assert_eq!(
                l.table_occurrence.to_string(),
                to.attribute("id").unwrap(),
                "{}",
                l.name
            );
            let parts: Vec<_> = x
                .descendants()
                .filter(|n| n.has_tag_name("Definition"))
                .collect();
            assert_eq!(l.parts.len(), parts.len(), "{}", l.name);
            for (p, xp) in l.parts.iter().zip(&parts) {
                assert_eq!(
                    p.kind.to_string(),
                    xp.attribute("kind").unwrap(),
                    "{}",
                    l.name
                );
                assert_eq!(
                    p.height.to_string(),
                    xp.attribute("size").unwrap(),
                    "{}",
                    l.name
                );
                assert_eq!(
                    p.top.to_string(),
                    xp.attribute("absolute").unwrap(),
                    "{}",
                    l.name
                );
            }
            for xo in x.descendants().filter(|n| n.has_tag_name("LayoutObject")) {
                let oid: u64 = xo.attribute("id").unwrap().parse().unwrap();
                let o = l
                    .objects
                    .iter()
                    .find(|o| o.id == oid)
                    .unwrap_or_else(|| panic!("{} object {oid}", l.name));
                assert_eq!(
                    o.kind.to_string(),
                    xo.attribute("kind").unwrap(),
                    "{} object {oid}",
                    l.name
                );
                let b = xo.children().find(|n| n.has_tag_name("Bounds")).unwrap();
                let a = |k: &str| b.attribute(k).unwrap().parse::<f64>().unwrap();
                assert_eq!(
                    (o.bounds.top, o.bounds.left, o.bounds.bottom, o.bounds.right),
                    (a("top"), a("left"), a("bottom"), a("right")),
                    "{} object {oid}",
                    l.name
                );
                let xparent = xo
                    .ancestors()
                    .skip(1)
                    .find(|n| n.has_tag_name("LayoutObject"))
                    .map(|n| n.attribute("id").unwrap().parse::<u64>().unwrap());
                if o.parent == xparent {
                    parents_ok += 1;
                } else {
                    eprintln!(
                        "parent mismatch: {} object {oid}: {:?} vs {xparent:?}",
                        l.name, o.parent
                    );
                }
                // ponytail: XML <Options> remaps some bits of `o.options`; mapping not decoded yet.
                objects_checked += 1;
            }
            layouts_checked += 1;
        }
    }
    eprintln!(
        "{layouts_checked} layouts, {objects_checked} layout objects checked against XML; parents {parents_ok}/{objects_checked}"
    );
    // Containers list their children (object field 6).
    assert_eq!(parents_ok, objects_checked, "nesting");
}

struct FileNames {
    tables: Vec<schema::Table>,
    tos: Vec<fmp12::graph::TableOccurrence>,
}

impl fmp12::calc::Names for FileNames {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        let bt = self.tos.iter().find(|t| t.id == to)?.base_table;
        Some(
            self.tables
                .iter()
                .find(|t| t.id == bt)?
                .fields
                .get(&field)?
                .name
                .clone(),
        )
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(self.tos.iter().find(|t| t.id == to)?.name.clone())
    }
}

#[test]
fn field_calculations_match_xml() {
    use std::collections::BTreeMap;
    let (mut ok, mut total) = (0, 0);
    let mut failures = Vec::new();
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let names = FileNames {
            tables: schema::tables(&file).unwrap(),
            tos: fmp12::graph::table_occurrences(&file).unwrap(),
        };
        // (table, field) -> (key4, key5)
        let mut calcs: BTreeMap<(u32, u32), (Vec<u8>, Vec<u8>)> = BTreeMap::new();
        file.walk(|path, c| {
            let (Some(p), fmp12::Chunk::Kv { key, data }) = (fmp12::int_path(path), c) else {
                return;
            };
            if let [t, 3, 5, f, 5] = p.as_slice() {
                let e = calcs.entry((*t, *f)).or_default();
                match key {
                    4 => e.0 = data.to_vec(),
                    5 => e.1.extend_from_slice(data),
                    _ => {}
                }
            }
        })
        .unwrap();
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
            for x in cat
                .descendants()
                .filter(|n| n.has_tag_name("Field") && n.attribute("datatype").is_some())
            {
                let fid: u32 = x.attribute("id").unwrap().parse().unwrap();
                let Some(want) = x
                    .descendants()
                    .find(|n| {
                        n.has_tag_name("Text")
                            && n.parent().is_some_and(|p| p.has_tag_name("Calculation"))
                    })
                    .and_then(|n| n.text())
                else {
                    continue;
                };
                let Some((k4, k5)) = calcs.get(&(tid, fid)) else {
                    continue;
                };
                total += 1;
                let ctx = fmp12::calc::context(k4).unwrap_or(0);
                let got = fmp12::calc::decompile(k5, ctx, &names).map(|s| s.replace('\r', "\n"));
                if got.as_deref() == Ok(want) {
                    ok += 1;
                } else {
                    failures.push(format!("{}: want {want:?}\n   got {got:?}", xml.display()));
                }
            }
        }
    }
    for f in failures.iter().take(8) {
        eprintln!("{f}");
    }
    eprintln!("{ok}/{total} field calculations decompiled exactly");
    assert_eq!(ok, total);
}

/// Checks one step's decoded parameters against its XML `<Step>`.
/// Returns the number of parameter values compared.
fn check_step(
    st: &fmp12::scripts::Step,
    xs: roxmltree::Node,
    names: &FileNames,
    ctx: &str,
) -> usize {
    use fmp12::scripts::{LayoutDest, SortOrder, StepParams, Target};
    let mut n = 0;
    let p = &st.raw_params;
    let xml_to = |n: roxmltree::Node| -> u32 {
        n.descendants()
            .find(|c| c.has_tag_name("TableOccurrenceReference"))
            .and_then(|c| c.attribute("id"))
            .and_then(|s| s.parse::<u32>().ok())
            .map_or(0, |v| v & 0xFFF)
    };
    let id_of = |n: roxmltree::Node| -> u32 { n.attribute("id").unwrap().parse().unwrap() };
    // Calculations by position.
    for c in xs
        .descendants()
        .filter(|c| c.has_tag_name("Calculation") && c.attribute("position").is_some())
    {
        let pos: u32 = c.attribute("position").unwrap().parse().unwrap();
        let want = c
            .descendants()
            .find(|t| t.has_tag_name("Text"))
            .and_then(|t| t.text())
            .unwrap_or("")
            .replace("\r\n", "\n");
        let got = p
            .calc(pos)
            .map(|k| k.text(names).unwrap().replace('\r', "\n"));
        match got {
            Some(g) => assert_eq!(g, want, "{ctx}: calc at {pos}"),
            // FileMaker shows the default repetition when none is stored.
            None => assert!(
                want == "1" || want.is_empty(),
                "{ctx}: missing calc at {pos} ({want})"
            ),
        }
        n += 1;
    }
    // Option bits shown as booleans.
    for b in xs.descendants().filter(|b| b.has_tag_name("Boolean")) {
        let Some(bit) = b.attribute("id").and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let mut set = st.options & bit != 0;
        if b.attribute("type") == Some("With dialog") {
            set = !set;
        }
        assert_eq!(
            set,
            b.attribute("value") == Some("True"),
            "{ctx}: {:?}",
            b.attribute("type")
        );
        n += 1;
    }
    let param = |ty: &str| {
        xs.descendants()
            .find(|c| c.has_tag_name("Parameter") && c.attribute("type") == Some(ty))
    };
    match &st.params {
        StepParams::Comment(t) => {
            let c = param("Comment").unwrap().first_element_child().unwrap();
            assert_eq!(
                t.replace('\r', "\n"),
                c.attribute("value").unwrap_or("").replace("\r\n", "\n"),
                "{ctx}"
            );
            n += 1;
        }
        StepParams::SetVariable { name, .. } => {
            let x = xs.descendants().find(|c| c.has_tag_name("Name")).unwrap();
            assert_eq!(name, x.attribute("value").unwrap(), "{ctx}");
            n += 1;
        }
        StepParams::SetField { field, .. } | StepParams::GoToField { field, .. } => {
            if let Some(x) = param("FieldReference").and_then(|x| x.first_element_child()) {
                let f = field.unwrap();
                assert_eq!((f.to, f.field), (xml_to(x), id_of(x)), "{ctx}");
                n += 1;
            } else {
                assert!(field.is_none(), "{ctx}");
            }
        }
        StepParams::InsertText { target, text, .. } => {
            let t = param("Text").unwrap().first_element_child().unwrap();
            assert_eq!(text, t.attribute("value").unwrap_or(""), "{ctx}");
            let x = param("Target").unwrap().first_element_child().unwrap();
            match target {
                Target::Variable(v) => assert_eq!(v, x.attribute("value").unwrap(), "{ctx}"),
                Target::Field(f) => assert_eq!((f.to, f.field), (xml_to(x), id_of(x)), "{ctx}"),
                Target::None => panic!("{ctx}: no target"),
            }
            n += 2;
        }
        StepParams::ShowCustomDialog { buttons, input, .. } => {
            for (i, b) in buttons.iter().enumerate() {
                let x = param(&format!("Button{}", i + 1)).unwrap();
                let commit = x.descendants().find(|c| c.has_tag_name("Boolean")).unwrap();
                assert_eq!(b.commit, commit.attribute("value") == Some("True"), "{ctx}");
                let label = b.label.as_ref().map(|c| c.text(names).unwrap());
                let want = x.attribute("value").map(|v| format!("\"{v}\""));
                assert_eq!(label, want, "{ctx}: button {}", i + 1);
                n += 2;
            }
            if let Some((f, _)) = input {
                let x = param("Field1").unwrap();
                let fr = x
                    .descendants()
                    .find(|c| c.has_tag_name("FieldReference"))
                    .unwrap();
                assert_eq!((f.to, f.field), (xml_to(fr), id_of(fr)), "{ctx}");
                n += 1;
            }
        }
        StepParams::GoToLayout { dest, .. } => {
            let x = param("LayoutReferenceContainer")
                .unwrap()
                .first_element_child()
                .unwrap();
            assert_eq!(st.arg.to_string(), x.attribute("value").unwrap(), "{ctx}");
            if let LayoutDest::Layout(id) = dest {
                let r = x.first_element_child().unwrap();
                assert_eq!(*id, id_of(r), "{ctx}");
            }
            n += 1;
        }
        StepParams::GoToRecord { .. } => {
            let x = param("Records").unwrap().first_element_child().unwrap();
            assert_eq!(st.arg.to_string(), x.attribute("value").unwrap(), "{ctx}");
            n += 1;
        }
        StepParams::PerformScript { script, .. } => {
            let r = xs
                .descendants()
                .find(|c| c.has_tag_name("ScriptReference"))
                .unwrap();
            assert_eq!(*script, Some(id_of(r)), "{ctx}");
            n += 1;
        }
        StepParams::Sort { keys, .. } => {
            let sorts: Vec<_> = xs
                .descendants()
                .filter(|c| c.has_tag_name("Sort"))
                .collect();
            assert_eq!(keys.len(), sorts.len(), "{ctx}");
            for ((f, o), x) in keys.iter().zip(&sorts) {
                let fr = x
                    .descendants()
                    .find(|c| c.has_tag_name("FieldReference"))
                    .unwrap();
                assert_eq!((f.to, f.field), (xml_to(fr), id_of(fr)), "{ctx}");
                let ty = match o {
                    SortOrder::Ascending => "Ascending",
                    SortOrder::Descending => "Descending",
                    SortOrder::Custom(v) => {
                        let vl = x
                            .descendants()
                            .find(|c| c.has_tag_name("ValueListReference"))
                            .unwrap();
                        assert_eq!(*v, id_of(vl), "{ctx}");
                        "Custom"
                    }
                };
                assert_eq!(ty, x.attribute("type").unwrap(), "{ctx}");
                n += 1;
            }
        }
        StepParams::Find { requests, .. } => {
            let xr: Vec<_> = xs
                .descendants()
                .filter(|c| c.has_tag_name("FindRequest"))
                .collect();
            assert_eq!(requests.len(), xr.len(), "{ctx}");
            for (r, x) in requests.iter().zip(&xr) {
                assert_eq!(r.omit, x.attribute("action") == Some("omit"), "{ctx}");
                let crit: Vec<_> = x.children().filter(|c| c.is_element()).collect();
                assert_eq!(r.criteria.len(), crit.len(), "{ctx}");
                for ((f, text), c) in r.criteria.iter().zip(&crit) {
                    let fr = c.first_element_child().unwrap();
                    assert_eq!((f.to, f.field), (xml_to(fr), id_of(fr)), "{ctx}");
                    assert_eq!(text, c.attribute("criteria").unwrap(), "{ctx}");
                    n += 1;
                }
            }
        }
        _ => {}
    }
    n
}

#[test]
fn step_parameters_match_xml() {
    let (mut steps, mut values, mut buttons) = (0, 0, 0);
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let names = FileNames {
            tables: schema::tables(&file).unwrap(),
            tos: fmp12::graph::table_occurrences(&file).unwrap(),
        };
        let scripts = fmp12::scripts::scripts(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
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
            let xsteps: Vec<_> = x.descendants().filter(|n| n.has_tag_name("Step")).collect();
            for (i, (st, xs)) in s.steps.iter().zip(&xsteps).enumerate() {
                assert_eq!(Some(xs.attribute("name").unwrap()), st.name(), "{}", s.name);
                let ctx = format!("{} / {} step {i} ({})", xml.display(), s.name, st.id);
                values += check_step(st, *xs, &names, &ctx);
                steps += 1;
            }
        }
        // Button actions.
        let layouts = fmp12::layouts::layouts(&file).unwrap();
        for xl in doc.descendants().filter(|n| n.has_tag_name("Layout")) {
            let lid: u32 = xl.attribute("id").unwrap().parse().unwrap();
            let Some(l) = layouts.iter().find(|l| l.id == lid) else {
                continue;
            };
            for xo in xl.descendants().filter(|n| n.has_tag_name("LayoutObject")) {
                let Some(action) = xo
                    .children()
                    .find(|c| c.has_tag_name("Button"))
                    .and_then(|b| b.children().find(|c| c.has_tag_name("action")))
                else {
                    continue;
                };
                let oid: u64 = xo.attribute("id").unwrap().parse().unwrap();
                let o = l.objects.iter().find(|o| o.id == oid).unwrap();
                let ctx = format!("{} layout {} object {oid}", xml.display(), l.name);
                let b = o.button.as_ref();
                if let Some(xs) = action.children().find(|c| c.has_tag_name("Step")) {
                    let b = b.unwrap_or_else(|| panic!("{ctx}: no action"));
                    assert_eq!(b.id.to_string(), xs.attribute("id").unwrap(), "{ctx}");
                    values += check_step(b, xs, &names, &ctx);
                } else if let Some(r) = action
                    .children()
                    .find(|c| c.has_tag_name("ScriptReference"))
                {
                    let b = b.unwrap_or_else(|| panic!("{ctx}: no action"));
                    let fmp12::scripts::StepParams::PerformScript { script, parameter } = &b.params
                    else {
                        panic!("{ctx}: {:?}", b.params)
                    };
                    assert_eq!(
                        script.map(|s| s.to_string()).as_deref(),
                        r.attribute("id"),
                        "{ctx}"
                    );
                    let want = action
                        .children()
                        .find(|c| c.has_tag_name("Calculation"))
                        .and_then(|c| c.descendants().find(|t| t.has_tag_name("Text")))
                        .and_then(|t| t.text());
                    let got = parameter.as_ref().map(|c| c.text(&names).unwrap());
                    assert_eq!(got.as_deref(), want, "{ctx}");
                } else {
                    continue;
                }
                buttons += 1;
            }
        }
    }
    eprintln!(
        "{steps} steps ({values} parameter values) and {buttons} button actions checked against XML"
    );
}

#[test]
fn security_matches_xml() {
    let mut checked = 0;
    for (fmp, xml) in pairs() {
        let file = fmp12::File::open(&fmp).unwrap();
        let accounts = fmp12::security::accounts(&file).unwrap();
        let sets = fmp12::security::privilege_sets(&file).unwrap();
        let text = std::fs::read_to_string(&xml).unwrap();
        let doc = roxmltree::Document::parse(&text).unwrap();
        for x in doc.descendants().filter(|n| n.has_tag_name("PrivilegeSet")) {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let s = sets.iter().find(|s| s.id == id).expect("privilege set");
            assert_eq!(s.name, x.attribute("name").unwrap());
            checked += 1;
        }
        for x in doc
            .descendants()
            .filter(|n| n.has_tag_name("Account") && n.attribute("type").is_some())
        {
            let id: u32 = x.attribute("id").unwrap().parse().unwrap();
            let a = accounts.iter().find(|a| a.id == id).expect("account");
            let name = x
                .descendants()
                .find(|n| n.has_tag_name("AccountName"))
                .and_then(|n| n.text())
                .unwrap();
            assert_eq!(a.name, name);
            let ps = x
                .children()
                .find(|n| n.has_tag_name("PrivilegeSetReference"))
                .unwrap();
            assert_eq!(
                a.privilege_set.to_string(),
                ps.attribute("id").unwrap(),
                "{name}"
            );
            checked += 1;
        }
    }
    eprintln!("{checked} accounts and privilege sets checked against XML");
}
