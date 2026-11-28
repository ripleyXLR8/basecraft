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

/// Clients (Id, Name, City) and Invoices (Id serial by 5, ClientId,
/// Amount, ClientName and ClientCity looked up), related by Clients::Id =
/// Invoices::ClientId, invoices sorted by amount (descending) and deleted
/// with their client; an occurrence "Big invoices" related by Id = ClientId
/// and Id < Amount. As in tools/oracle/graph_demo.sh, checked in FileMaker.
struct Shop {
    file: fmp12::File,
    db: Database,
    clients: u32,
    invoices: u32,
    cto: u32,
    f: std::collections::BTreeMap<&'static str, u32>,
}

fn shop() -> Shop {
    use fmp12::schema::{Lookup, NoMatch};
    use fmp12::schema_edit::{RelSide, RelationshipSpec};
    let mut file =
        fmp12::File::from_bytes(fmp12::schema_edit::new_file("Clients").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let c = db.tables[0].id;
    let mut f = std::collections::BTreeMap::new();
    for (n, t) in [
        ("Id", DataType::Number),
        ("Name", DataType::Text),
        ("City", DataType::Text),
    ] {
        f.insert(
            n,
            db.add_field(&mut file, c, &FieldSpec::new(n, t)).unwrap(),
        );
    }
    let i = db.add_table(&mut file, "Invoices").unwrap();
    let mut id = FieldSpec::new("Id", DataType::Number);
    id.auto_enter.serial = Some("1".into());
    id.auto_enter.serial_increment = 5;
    f.insert("InvoiceId", db.add_field(&mut file, i, &id).unwrap());
    for n in ["ClientId", "Amount"] {
        f.insert(
            n,
            db.add_field(&mut file, i, &FieldSpec::new(n, DataType::Number))
                .unwrap(),
        );
    }
    let to = |db: &Database, n: &str| db.occurrences.iter().find(|o| o.name == n).unwrap().id;
    let (cto, ito) = (to(&db, "Clients"), to(&db, "Invoices"));
    let mut rel = RelationshipSpec::equal(cto, f["Id"], ito, f["ClientId"]);
    rel.right_options = RelSide {
        create: true,
        delete: true,
        sort: vec![(f["Amount"], true)],
    };
    db.add_relationship(&mut file, &rel).unwrap();
    let big = db.add_occurrence(&mut file, i, "Big invoices").unwrap();
    let mut rel = RelationshipSpec::equal(cto, f["Id"], big, f["ClientId"]);
    rel.predicates.push(fmp12::graph::Predicate {
        op: fmp12::graph::JoinOp::Less,
        left_field: f["Id"],
        right_field: f["Amount"],
    });
    db.add_relationship(&mut file, &rel).unwrap();
    for (n, src, no_match) in [
        ("ClientName", "Name", NoMatch::Constant("unknown".into())),
        ("ClientCity", "City", NoMatch::Lower),
    ] {
        let mut l = FieldSpec::new(n, DataType::Text);
        l.auto_enter.lookup = Some(Lookup {
            context: ito,
            from: cto,
            field: f[src],
            no_match,
            copy_empty: false,
        });
        f.insert(n, db.add_field(&mut file, i, &l).unwrap());
    }
    Shop {
        file,
        db,
        clients: c,
        invoices: i,
        cto,
        f,
    }
}

impl Shop {
    fn client(&mut self, id: &str, name: &str, city: &str) -> usize {
        let r = self.db.new_record(self.clients).unwrap();
        let v = [
            (self.f["Id"], id.to_string()),
            (self.f["Name"], name.into()),
            (self.f["City"], city.into()),
        ];
        self.db.set_values(self.clients, r, &v);
        r
    }
    fn invoice(&mut self, client: &str, amount: &str) -> usize {
        let r = self.db.new_record(self.invoices).unwrap();
        let v = [
            (self.f["ClientId"], client.to_string()),
            (self.f["Amount"], amount.into()),
        ];
        self.db.set_values(self.invoices, r, &v);
        r
    }
    fn get(&self, table: u32, r: usize, field: &str) -> String {
        self.db.value(table, r, self.f[field]).to_text()
    }
}

#[test]
fn related_records_through_new_relationships() {
    let mut s = shop();
    let ada = s.client("1", "Ada", "Paris");
    s.client("2", "Bob", "Lyon");
    for (c, a) in [("1", "5"), ("1", "20"), ("2", "1"), ("1", "1")] {
        s.invoice(c, a);
    }
    // Sorted by amount, descending; the second relationship also needs Id < Amount.
    let all = |src: &str| s.db.eval_source(src, s.clients, s.cto, Some(ada)).to_text();
    assert_eq!(all("List ( Invoices::Amount )"), "20\r5\r1");
    assert_eq!(all("Count ( Invoices::Amount )"), "3");
    assert_eq!(all("List ( ${Big invoices}::Amount )"), "5\r20");
    assert_eq!(all("Invoices::Amount"), "20");
    // Serial numbers step by 5.
    let ids: Vec<String> = (0..4).map(|r| s.get(s.invoices, r, "InvoiceId")).collect();
    assert_eq!(ids, ["1", "6", "11", "16"]);
}

#[test]
fn lookups_copy_when_the_match_field_changes() {
    let mut s = shop();
    s.client("1", "Ada", "Paris");
    s.client("2", "Bob", "Lyon");
    let r = s.invoice("2", "10");
    assert_eq!(s.get(s.invoices, r, "ClientName"), "Bob");
    assert_eq!(s.get(s.invoices, r, "ClientCity"), "Lyon");
    // No match: the constant, and the next lower match value's city.
    s.db.set_values(s.invoices, r, &[(s.f["ClientId"], "7".into())]);
    assert_eq!(s.get(s.invoices, r, "ClientName"), "unknown");
    assert_eq!(s.get(s.invoices, r, "ClientCity"), "Lyon");
    // Changing another field does not look up again.
    let i = s.db.tables.iter().position(|t| t.id == s.invoices).unwrap();
    s.db.tables[i].records[r]
        .values
        .insert(s.f["ClientName"], "kept".into());
    s.db.set_values(s.invoices, r, &[(s.f["Amount"], "3".into())]);
    assert_eq!(s.get(s.invoices, r, "ClientName"), "kept");
    s.db.set_values(s.invoices, r, &[(s.f["ClientId"], "1".into())]);
    assert_eq!(s.get(s.invoices, r, "ClientName"), "Ada");
    // Looked-up values are saved like typed ones.
    let file = fmp12::File::from_bytes(s.db.save(&s.file).unwrap()).unwrap();
    let db = Database::load(&file).unwrap();
    let t = db.table(s.invoices).unwrap();
    assert_eq!(t.records[0].values[&s.f["ClientName"]], "Ada");
}

#[test]
fn deleting_a_client_deletes_its_invoices() {
    let mut s = shop();
    let ada = s.client("1", "Ada", "Paris");
    s.client("2", "Bob", "Lyon");
    s.invoice("1", "5");
    s.invoice("2", "6");
    s.invoice("1", "7");
    let gone = s.db.delete_record(s.clients, ada);
    assert_eq!(gone.len(), 3);
    let inv = s.db.table(s.invoices).unwrap();
    assert_eq!(inv.records.len(), 1);
    assert_eq!(inv.records[0].values[&s.f["Amount"]], "6");
    // Deleting an invoice leaves its client (no delete option on the left).
    s.db.delete_record(s.invoices, 0);
    assert_eq!(s.db.table(s.clients).unwrap().records.len(), 1);
}

#[test]
fn value_lists_through_the_engine() {
    use fmp12::valuelists::Source;
    let mut s = shop();
    s.client("2", "Bob", "Lyon");
    let ada = s.client("1", "Ada", "Paris");
    s.invoice("1", "5");
    s.invoice("2", "6");
    s.invoice("1", "7");
    let ito =
        s.db.occurrences
            .iter()
            .find(|o| o.name == "Invoices")
            .unwrap()
            .id;
    let custom =
        s.db.add_value_list(&mut s.file, "Cities", &Source::Custom("Paris\rLyon".into()))
            .unwrap();
    let names = Source::Field {
        to: s.cto,
        field: s.f["Id"],
        second: Some((s.cto, s.f["Name"])),
        second_only: true,
        sort_by_second: true,
        related_from: None,
    };
    let by_name = s.db.add_value_list(&mut s.file, "Ids", &names).unwrap();
    let related = Source::Field {
        to: ito,
        field: s.f["Amount"],
        second: None,
        second_only: false,
        sort_by_second: false,
        related_from: Some(s.cto),
    };
    let amounts =
        s.db.add_value_list(&mut s.file, "Amounts", &related)
            .unwrap();
    assert_eq!(s.db.value_list_values(custom), ["Paris", "Lyon"]);
    // Ids sorted by the names they show (Ada 1, Bob 2).
    assert_eq!(s.db.value_list_values(by_name), ["1", "2"]);
    assert_eq!(
        s.db.value_list_values_from(amounts, Some((s.cto, ada))),
        ["5", "7"]
    );
    assert!(s.db.value_list_values_from(amounts, None).is_empty());
    s.db.delete_value_list(&mut s.file, custom).unwrap();
    assert_eq!(s.db.value_lists.len(), 2);
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

#[test]
fn relookup_copies_the_related_values_again() {
    let mut s = shop();
    let ada = s.client("1", "Ada", "Paris");
    let r = s.invoice("1", "10");
    let other = s.invoice("1", "20");
    // The client's name changes: looked-up values keep the old one…
    let c = s.db.tables.iter().position(|t| t.id == s.clients).unwrap();
    s.db.tables[c].records[ada]
        .values
        .insert(s.f["Name"], "Ada L.".into());
    assert_eq!(s.get(s.invoices, r, "ClientName"), "Ada");
    // …until Relookup Field Contents on the match field.
    s.db.relookup(s.invoices, &[r], s.f["ClientId"]);
    assert_eq!(s.get(s.invoices, r, "ClientName"), "Ada L.");
    assert_eq!(s.get(s.invoices, other, "ClientName"), "Ada");
    // A field no lookup matches on changes nothing.
    s.db.relookup(s.invoices, &[other], s.f["Amount"]);
    assert_eq!(s.get(s.invoices, other, "ClientName"), "Ada");
}

#[test]
fn value_list_size_and_custom_message_validation() {
    use engine::Rule;
    use fmp12::valuelists::Source;
    let mut s = shop();
    let sizes =
        s.db.add_value_list(&mut s.file, "Sizes", &Source::Custom("S\rM\rL".into()))
            .unwrap();
    let mut size = FieldSpec::new("Size", DataType::Text);
    size.validation.value_list = Some(sizes);
    let size = s.db.add_field(&mut s.file, s.invoices, &size).unwrap();
    let mut code = FieldSpec::new("Code", DataType::Text);
    code.validation.max_chars = Some(3);
    code.validation.message = Some("\"At most \" & 3 & \" characters\"".into());
    let code = s.db.add_field(&mut s.file, s.invoices, &code).unwrap();
    let r = s.invoice("1", "5");
    let rules = |s: &Shop| -> Vec<(u32, Rule, u32)> {
        s.db.validate(s.invoices, r)
            .iter()
            .map(|e| (e.field, e.rule, e.rule.code()))
            .collect()
    };
    assert!(rules(&s).is_empty());
    s.db.set_values(s.invoices, r, &[(size, "XL".into()), (code, "ABCD".into())]);
    assert_eq!(
        rules(&s),
        [(size, Rule::ValueList, 506), (code, Rule::MaxChars, 511)]
    );
    // Value list values match whatever the case; the message is a calculation.
    s.db.set_values(s.invoices, r, &[(size, "m".into()), (code, "ABC".into())]);
    assert!(rules(&s).is_empty());
    assert_eq!(s.db.validation_message(s.invoices, r, size), None);
    assert_eq!(
        s.db.validation_message(s.invoices, r, code).as_deref(),
        Some("At most 3 characters")
    );
}

/// "In range" (503) and "validated by calculation" (507, `Self` is the
/// value), checked in the dialog's order (range before calculation before
/// maximum characters); "always" reported for imports and skipped commits.
#[test]
fn range_and_calculation_validation() {
    use engine::Rule;
    let mut s = shop();
    let mut qty = FieldSpec::new("Qty", DataType::Number);
    qty.validation.range = Some(("1".into(), "10".into()));
    qty.validation.calculation = Some("Mod ( Self ; 2 ) = 0".into());
    qty.validation.max_chars = Some(1);
    qty.validation.always = true;
    qty.validation.allow_override = false;
    let qty = s.db.add_field(&mut s.file, s.invoices, &qty).unwrap();
    let mut day = FieldSpec::new("Day", DataType::Date);
    day.validation.range = Some(("1/1/2020".into(), "31/12/2020".into()));
    let day = s.db.add_field(&mut s.file, s.invoices, &day).unwrap();
    // A calculation that runs on empty values too.
    let mut note = FieldSpec::new("Note", DataType::Text);
    note.validation.calculation = Some("not IsEmpty ( Self ) or Amount < 100".into());
    let note = s.db.add_field(&mut s.file, s.invoices, &note).unwrap();
    let r = s.invoice("1", "5");
    let rules = |s: &Shop| -> Vec<(u32, Rule, u32)> {
        s.db.validate(s.invoices, r)
            .iter()
            .map(|e| (e.field, e.rule, e.rule.code()))
            .collect()
    };
    assert!(rules(&s).is_empty());
    let set = |s: &mut Shop, f: u32, v: &str| s.db.set_values(s.invoices, r, &[(f, v.into())]);
    set(&mut s, qty, "12");
    assert_eq!(rules(&s), [(qty, Rule::Range, 503)]);
    set(&mut s, qty, "3");
    assert_eq!(rules(&s), [(qty, Rule::Calculation, 507)]);
    set(&mut s, qty, "4");
    assert!(rules(&s).is_empty());
    set(&mut s, qty, "10");
    assert_eq!(rules(&s), [(qty, Rule::MaxChars, 511)]);
    let e = s.db.validate(s.invoices, r)[0];
    assert!(e.always && !e.can_override);
    set(&mut s, qty, "");
    set(&mut s, day, "15/6/2021");
    assert_eq!(rules(&s), [(day, Rule::Range, 503)]);
    set(&mut s, day, "15/6/2020");
    assert!(rules(&s).is_empty());
    let amount = s.f["Amount"];
    set(&mut s, amount, "500");
    assert_eq!(rules(&s), [(note, Rule::Calculation, 507)]);
    set(&mut s, note, "big order");
    assert!(rules(&s).is_empty());
}

/// Auto-enter calculations see `Self` and evaluate again when a field
/// they use changes: over the value when "Do not replace existing value"
/// is off, only into an empty field when it is on.
#[test]
fn auto_enter_self_and_reevaluation() {
    let mut s = shop();
    let mut total = FieldSpec::new("WithTax", DataType::Number);
    total.auto_enter.calculation = Some("Amount * 2".into());
    total.auto_enter.replace_existing = true;
    let total = s.db.add_field(&mut s.file, s.invoices, &total).unwrap();
    let mut first = FieldSpec::new("FirstAmount", DataType::Number);
    first.auto_enter.calculation = Some("If ( IsEmpty ( Self ) ; Amount ; Self )".into());
    let first = s.db.add_field(&mut s.file, s.invoices, &first).unwrap();
    let r = s.invoice("1", "5");
    let get = |s: &Shop, f: u32| s.db.value(s.invoices, r, f).to_text();
    assert_eq!(get(&s, total), "10");
    assert_eq!(get(&s, first), "5");
    let amount = s.f["Amount"];
    s.db.set_values(s.invoices, r, &[(amount, "7".into())]);
    assert_eq!(get(&s, total), "14");
    assert_eq!(get(&s, first), "5");
}

#[test]
fn summary_options_written_and_computed() {
    use fmp12::schema::SummaryOp;
    use fmp12::schema_edit::FieldType;
    let mut s = shop();
    for (c, a) in [("1", "10"), ("1", "5"), ("2", "30")] {
        s.invoice(c, a);
    }
    let amount = s.f["Amount"];
    let summary = |name: &str, op, running| FieldSpec {
        field_type: FieldType::Summary {
            op,
            field: amount,
            running,
            by: None,
        },
        ..FieldSpec::new(name, DataType::Number)
    };
    let pop = summary("Pop", SummaryOp::StandardDeviation, true);
    let pop = s.db.add_field(&mut s.file, s.invoices, &pop).unwrap();
    let list = summary("List", SummaryOp::List, false);
    let list = s.db.add_field(&mut s.file, s.invoices, &list).unwrap();
    let running = summary("Running", SummaryOp::Total, true);
    let running = s.db.add_field(&mut s.file, s.invoices, &running).unwrap();
    let t = s.db.table(s.invoices).unwrap();
    assert!(t.field(pop).unwrap().def.summary_population());
    let op = t.field(list).unwrap().def.summary().unwrap().op;
    assert_eq!(op, SummaryOp::List);
    // Population standard deviation of 10, 5, 30: sqrt(350 / 3).
    let v = s.db.value(s.invoices, 0, pop).to_text();
    assert!(v.starts_with("10.801234497"), "{v}");
    assert_eq!(s.db.value(s.invoices, 0, list).to_text(), "10\r5\r30");
    // A running total in table order (the engine has no found set).
    let run: Vec<String> = (0..3)
        .map(|r| s.db.value(s.invoices, r, running).to_text())
        .collect();
    assert_eq!(run, ["10", "15", "45"]);
    // The options survive a save and a reload.
    let file = fmp12::File::from_bytes(s.db.save(&s.file).unwrap()).unwrap();
    let db = Database::load(&file).unwrap();
    let t = db.table(s.invoices).unwrap();
    assert!(t.field(pop).unwrap().def.summary_population());
}

/// File › Import Records from another `.fmp12`: its tables as rows, then
/// imported by field name.
#[test]
fn import_from_fmp12_file() {
    let mut file = fmp12::File::from_bytes(fmp12::schema_edit::new_file("Notes").unwrap()).unwrap();
    let mut db = Database::load(&file).unwrap();
    let t = db.add_table(&mut file, "People").unwrap();
    let name = db
        .add_field(&mut file, t, &FieldSpec::new("Name", DataType::Text))
        .unwrap();
    let born = db
        .add_field(&mut file, t, &FieldSpec::new("Born", DataType::Date))
        .unwrap();
    for (n, b) in [("Ada", "12/10/1815"), ("Bob", "")] {
        let r = db.new_record(t).unwrap();
        db.set_values(t, r, &[(name, n.into()), (born, b.into())]);
    }
    let bytes = db.save(&file).unwrap();
    let tables = engine::import::fmp12_source(&bytes, &db.locale).unwrap();
    let (_, rows) = tables.iter().find(|(n, _)| n == "People").unwrap();
    assert_eq!(rows[0], ["Name", "Born"]);
    assert_eq!(rows[1], ["Ada", "12/10/1815"]);
    // Into a copy without records.
    let mut dst = Database::load(&fmp12::File::from_bytes(bytes).unwrap()).unwrap();
    dst.tables
        .iter_mut()
        .find(|x| x.id == t)
        .unwrap()
        .records
        .clear();
    let o = engine::import::Options {
        format: engine::export::Format::Tab,
        first_row_names: true,
        mapping: engine::import::Mapping::ByName,
        mode: engine::import::Mode::Add,
        auto_enter: true,
    };
    let rep = engine::import::import(&mut dst, t, &[], rows.clone(), &o).unwrap();
    assert_eq!(rep.added.len(), 2);
    assert_eq!(dst.value(t, 1, name).to_text(), "Bob");
}
