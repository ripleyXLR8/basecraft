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

/// Clients and Invoices on a new file. Returns the editor and (clients,
/// its occurrence, invoices, its occurrence).
fn shop() -> (Editor, [u32; 4]) {
    let file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Clients").unwrap()).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let c = ed.table_id("Clients").unwrap();
    for (n, d) in [
        ("Id", DataType::Number),
        ("Name", DataType::Text),
        ("City", DataType::Text),
    ] {
        ed.add_field(c, &FieldSpec::new(n, d)).unwrap();
    }
    let (i, ito, _) = ed.add_table("Invoices").unwrap();
    for (n, d) in [
        ("Id", DataType::Number),
        ("ClientId", DataType::Number),
        ("Amount", DataType::Number),
    ] {
        ed.add_field(i, &FieldSpec::new(n, d)).unwrap();
    }
    let cto = ed.occurrence_id("Clients").unwrap();
    (ed, [c, cto, i, ito])
}

#[test]
fn occurrences_relationships_and_value_lists_round_trip() {
    use fmp12::graph::{JoinOp, Predicate};
    use fmp12::schema_edit::{RelSide, RelationshipSpec};
    use fmp12::valuelists::Source;
    let (mut ed, [c, cto, i, ito]) = shop();
    let f = |ed: &Editor, t: u32, n: &str| ed.field_id(t, n).unwrap();
    let (cid, name, city) = (f(&ed, c, "Id"), f(&ed, c, "Name"), f(&ed, c, "City"));
    let (cl, amount) = (f(&ed, i, "ClientId"), f(&ed, i, "Amount"));
    // Clients -< Invoices: create and delete invoices, sorted by amount (desc).
    let mut spec = RelationshipSpec::equal(cto, cid, ito, cl);
    spec.right_options = RelSide {
        create: true,
        delete: true,
        sort: vec![(amount, true)],
    };
    spec.left_options.sort = vec![(name, false)];
    let r1 = ed.add_relationship(&spec).unwrap();
    // A second occurrence of Invoices, named differently, with every operator.
    let big = ed.add_occurrence(i, "Big invoices").unwrap();
    let ops = [
        JoinOp::Equal,
        JoinOp::NotEqual,
        JoinOp::Less,
        JoinOp::LessOrEqual,
        JoinOp::Greater,
        JoinOp::GreaterOrEqual,
        JoinOp::Cartesian,
    ];
    let preds: Vec<Predicate> = ops
        .iter()
        .map(|op| Predicate {
            op: *op,
            left_field: cid,
            right_field: amount,
        })
        .collect();
    let mut spec2 = RelationshipSpec::equal(cto, cid, big, amount);
    spec2.predicates = preds.clone();
    spec2.left_options.delete = true;
    // Creation needs = only; a duplicate pair or a cycle is refused.
    let mut bad = spec2.clone();
    bad.right_options.create = true;
    assert!(ed.add_relationship(&bad).is_err());
    let r2 = ed.add_relationship(&spec2).unwrap();
    assert!(ed.add_relationship(&spec).is_err());
    assert!(
        ed.add_relationship(&RelationshipSpec::equal(ito, cl, big, amount))
            .is_err()
    );
    // Value lists: custom, two fields, related only.
    let cities = ed
        .add_value_list("Cities", &Source::Custom("Paris\rLyon\rNice".into()))
        .unwrap();
    let names_src = Source::Field {
        to: cto,
        field: cid,
        second: Some((cto, name)),
        second_only: true,
        sort_by_second: true,
        related_from: None,
    };
    let names = ed.add_value_list("Client names", &names_src).unwrap();
    let related = Source::Field {
        to: ito,
        field: amount,
        second: None,
        second_only: false,
        sort_by_second: false,
        related_from: Some(cto),
    };
    let rel_vl = ed.add_value_list("Amounts", &related).unwrap();
    assert!(
        ed.add_value_list("cities", &Source::Custom("x".into()))
            .is_err()
    );
    // Rename an occurrence, then reopen.
    ed.rename_occurrence(big, "Large").unwrap();
    let file = reopen(ed.finish());

    let tos = fmp12::graph::table_occurrences(&file).unwrap();
    let large = tos.iter().find(|o| o.id == big).unwrap();
    assert_eq!((large.name.as_str(), large.base_table), ("Large", i));
    assert!(large.width > 0.0 && large.left > 0.0);
    let rels = fmp12::graph::relationships(&file).unwrap();
    let a = rels.iter().find(|r| r.id == r1).unwrap();
    assert_eq!((a.left, a.right), (cto, ito));
    assert!(a.right_cascade_create() && a.right_cascade_delete());
    assert!(!a.left_cascade_create() && !a.left_cascade_delete());
    assert_eq!(a.right_sort.len(), 1);
    assert!(a.right_sort[0].descending && a.right_sort[0].field == amount);
    assert_eq!(a.left_sort[0].field, name);
    let b = rels.iter().find(|r| r.id == r2).unwrap();
    assert_eq!(b.predicates, preds);
    assert!(b.left_cascade_delete());
    let vls = fmp12::valuelists::value_lists(&file).unwrap();
    let by = |id: u32| vls.iter().find(|v| v.id == id).unwrap();
    assert_eq!(
        by(cities).source,
        Source::Custom("Paris\rLyon\rNice".into())
    );
    assert_eq!(by(names).source, names_src);
    assert_eq!(by(names).name, "Client names");
    assert_eq!(by(rel_vl).source, related);

    // Edit and delete.
    let mut ed = Editor::open(&file).unwrap();
    let mut s = ed.relationship_spec(r1).unwrap();
    assert_eq!(s, spec);
    s.predicates.push(Predicate {
        op: JoinOp::Equal,
        left_field: city,
        right_field: amount,
    });
    s.right_options.sort.clear();
    ed.set_relationship(r1, &s).unwrap();
    ed.set_value_list(cities, "Towns", &Source::Custom("Oslo".into()))
        .unwrap();
    ed.delete_value_list(names).unwrap();
    ed.delete_relationship(r2).unwrap();
    // The last occurrence of a table stays.
    assert!(ed.delete_occurrence(cto).is_err());
    ed.delete_occurrence(big).unwrap();
    let file = reopen(ed.finish());
    let rels = fmp12::graph::relationships(&file).unwrap();
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].predicates.len(), 2);
    assert!(rels[0].right_sort.is_empty());
    assert!(
        !fmp12::graph::table_occurrences(&file)
            .unwrap()
            .iter()
            .any(|o| o.id == big)
    );
    let vls = fmp12::valuelists::value_lists(&file).unwrap();
    assert_eq!(vls.len(), 2);
    assert!(
        vls.iter()
            .any(|v| v.name == "Towns" && v.source == Source::Custom("Oslo".into()))
    );
}

#[test]
fn lookups_and_more_field_options_round_trip() {
    use fmp12::schema::{Lookup, NoMatch};
    use fmp12::schema_edit::RelationshipSpec;
    use fmp12::valuelists::Source;
    let (mut ed, [c, cto, i, ito]) = shop();
    let f = |ed: &Editor, t: u32, n: &str| ed.field_id(t, n).unwrap();
    let (cid, name) = (f(&ed, c, "Id"), f(&ed, c, "Name"));
    let (cl, amount) = (f(&ed, i, "ClientId"), f(&ed, i, "Amount"));
    let vl = ed
        .add_value_list("Cities", &Source::Custom("Paris\rLyon".into()))
        .unwrap();
    // A lookup needs a relationship.
    let mut look = FieldSpec::new("Client name", DataType::Text);
    look.auto_enter.lookup = Some(Lookup {
        context: ito,
        from: cto,
        field: name,
        no_match: NoMatch::Constant("?".into()),
        copy_empty: false,
    });
    assert!(ed.add_field(i, &look).is_err());
    ed.add_relationship(&RelationshipSpec::equal(cto, cid, ito, cl))
        .unwrap();
    let lk = ed.add_field(i, &look).unwrap();
    let mut lower = look.clone();
    lower.name = "Lower".into();
    if let Some(l) = lower.auto_enter.lookup.as_mut() {
        l.no_match = NoMatch::Lower;
        l.copy_empty = true;
    }
    let lw = ed.add_field(i, &lower).unwrap();
    let mut serial = FieldSpec::new("Number", DataType::Number);
    serial.auto_enter.serial = Some("100".into());
    serial.auto_enter.serial_increment = 10;
    let sr = ed.add_field(i, &serial).unwrap();
    let mut town = FieldSpec::new("Town", DataType::Text);
    town.validation.value_list = Some(vl);
    town.validation.max_chars = Some(12);
    town.validation.message = Some("\"Pick a city\"".into());
    let tw = ed.add_field(c, &town).unwrap();
    let summary = |name: &str, op: SummaryOp, running: bool| FieldSpec {
        field_type: FieldType::Summary {
            op,
            field: amount,
            running,
            by: Some(cl),
        },
        ..FieldSpec::summary(name, op, amount)
    };
    let running = summary("Running", SummaryOp::Total, true);
    let rn = ed.add_field(i, &running).unwrap();
    let weighted = summary("Weighted", SummaryOp::Average, false);
    let wa = ed.add_field(i, &weighted).unwrap();
    let share = summary("Share", SummaryOp::FractionOfTotal, false);
    let fr = ed.add_field(i, &share).unwrap();
    // A restart needs a running total; a minimum takes no second field.
    assert!(
        ed.add_field(i, &summary("Bad", SummaryOp::Total, false))
            .is_err()
    );
    assert!(
        ed.add_field(i, &summary("Bad", SummaryOp::Minimum, false))
            .is_err()
    );

    let file = reopen(ed.finish());
    let tables = fmp12::schema::tables(&file).unwrap();
    let inv = tables.iter().find(|t| t.id == i).unwrap();
    let l = inv.fields[&lk].lookup().unwrap();
    assert_eq!((l.context, l.from, l.field), (ito, cto, name));
    assert_eq!(l.no_match, NoMatch::Constant("?".into()));
    assert!(!l.copy_empty);
    let l = inv.fields[&lw].lookup().unwrap();
    assert_eq!(l.no_match, NoMatch::Lower);
    assert!(l.copy_empty);
    assert_eq!(inv.fields[&sr].serial_increment(), 10);
    assert!(inv.fields[&rn].summary_running());
    assert_eq!(inv.fields[&rn].summary_by(), Some(cl));
    assert_eq!(inv.fields[&wa].summary_by(), Some(cl));
    assert_eq!(inv.fields[&fr].summary_by(), Some(cl));
    let cli = tables.iter().find(|t| t.id == c).unwrap();
    let t = &cli.fields[&tw];
    assert_eq!(t.validation_value_list(), Some(vl));
    assert_eq!(t.max_size(), Some(12));
    assert!(t.validation().custom_message);
    // The editor reads every option back.
    let ed = Editor::open(&file).unwrap();
    assert_eq!(ed.field_spec(i, lk).unwrap(), look);
    assert_eq!(ed.field_spec(i, lw).unwrap(), lower);
    assert_eq!(ed.field_spec(i, sr).unwrap(), serial);
    assert_eq!(ed.field_spec(c, tw).unwrap(), town);
    assert_eq!(ed.field_spec(i, rn).unwrap(), running);
    assert_eq!(ed.field_spec(i, wa).unwrap(), weighted);
    assert_eq!(ed.field_spec(i, fr).unwrap(), share);
}

/// Names kept as long values in `[node].[16]` (FileMaker's Japanese,
/// Korean and Chinese templates): the editor finds and renames them.
#[test]
fn long_form_names() {
    use fmp12::save::{encode_text, path_elem};
    use fmp12::write::{Entry, Item, encode};
    let file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("T").unwrap()).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let (t, _, _) = ed.add_table("People").unwrap();
    let f = ed
        .add_field(t, &FieldSpec::new("Name", DataType::Text))
        .unwrap();
    let file = reopen(ed.finish());
    // Moves every name into its long form, as FileMaker writes them.
    let mut es = fmp12::write::entries(&file).unwrap();
    for name in ["People", "Name"] {
        let inline = encode(&Item::Kv {
            key: 16,
            data: encode_text(name),
        });
        let text = encode_text(name);
        let (a, b) = text.split_at(2);
        while let Some(at) = es.iter().position(|e| e.item == inline) {
            let sub = [es.remove(at).path, vec![path_elem(16)]].concat();
            es.push(Entry {
                path: sub.clone(),
                item: vec![0x80],
            });
            for (key, data) in [(1, a.to_vec()), (2, b.to_vec())] {
                es.push(Entry {
                    path: sub.clone(),
                    item: encode(&Item::Kv { key, data }),
                });
            }
        }
    }
    fmp12::write::sort(&mut es);
    let file = reopen(fmp12::write::build(&file.bytes()[..fmp12::SECTOR], &es));
    let mut ed = Editor::open(&file).unwrap();
    assert_eq!(ed.table_id("People"), Some(t));
    assert_eq!(ed.field_id(t, "Name"), Some(f));
    ed.rename_table(t, "Folks").unwrap();
    ed.rename_field(t, f, "Nom").unwrap();
    let file = reopen(ed.finish());
    let tables = fmp12::schema::tables(&file).unwrap();
    let tb = tables.iter().find(|x| x.id == t).unwrap();
    assert_eq!(tb.name, "Folks");
    assert_eq!(tb.fields[&f].name, "Nom");
    // The occurrence and layout named after the table follow it.
    let tos = fmp12::graph::table_occurrences(&file).unwrap();
    assert!(tos.iter().any(|o| o.name == "Folks"));
    let lays = fmp12::layouts::layouts(&file).unwrap();
    assert!(lays.iter().any(|l| l.name == "Folks"));
    assert!(!lays.iter().any(|l| l.name.contains("People")));
}
