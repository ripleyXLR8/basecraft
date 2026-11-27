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
