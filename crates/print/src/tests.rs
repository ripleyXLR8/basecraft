use super::*;
use fmp12::layouts::{Bounds, Part};
use fmp12::schema::{Summary, SummaryOp};

const T: u32 = 130;
const NAME: u32 = 1;
const CITY: u32 = 2;
const AMOUNT: u32 = 3;
const TOTAL: u32 = 4;

fn field(id: u32, name: &str, kind: u8, dt: u8) -> engine::Field {
    let def = fmp12::schema::Field {
        name: name.into(),
        options: vec![kind, dt],
        ..Default::default()
    };
    engine::Field {
        id,
        name: name.into(),
        kind: def.kind(),
        data_type: def.data_type(),
        calc: None,
        auto_calc: None,
        context: 0,
        def,
    }
}

fn obj(id: u64, top: f64, left: f64, f: Option<u32>, text: Option<&str>) -> LayoutObject {
    LayoutObject {
        id,
        kind: if f.is_some() { 1 } else { 2 },
        bounds: Bounds {
            top,
            left,
            bottom: top + 18.0,
            right: left + 120.0,
        },
        field: f.map(|f| (0x104000 | T as u64, f as u64, 1)),
        text: text.map(str::to_string),
        ..Default::default()
    }
}

fn part(kind: u64, top: f64, height: f64) -> Part {
    Part {
        kind,
        top,
        height,
        ..Default::default()
    }
}

/// People with a report layout: header, sub-summary by City, body, grand
/// summary, footer.
fn db(rows: &[(&str, &str, &str)]) -> Database {
    let mut total = field(TOTAL, "Total", 3, 2);
    total.def.summary_raw = fmp12::schema::Field::summary_bytes(Summary {
        op: SummaryOp::Total,
        field: AMOUNT,
    });
    let mut t = engine::Table::new(
        T,
        "People",
        vec![
            field(NAME, "Name", 1, 1),
            field(CITY, "City", 1, 1),
            field(AMOUNT, "Amount", 1, 2),
            total,
        ],
    );
    for (n, c, a) in rows {
        let r = t.new_record();
        t.records[r].values = [(NAME, n), (CITY, c), (AMOUNT, a)]
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k, v.to_string()))
            .collect();
    }
    let mut sub = part(part::LEADING_SUB_SUMMARY, 40.0, 20.0);
    sub.break_field = Some((T, CITY));
    let layout = Layout {
        id: 1,
        name: "Report".into(),
        table_occurrence: 0x104000 | T as u64,
        width: 500.0,
        parts: vec![
            part(part::HEADER, 0.0, 40.0),
            sub,
            part(part::BODY, 60.0, 20.0),
            part(part::TRAILING_GRAND_SUMMARY, 80.0, 20.0),
            part(part::FOOTER, 100.0, 20.0),
        ],
        objects: vec![
            obj(1, 10.0, 0.0, None, Some("People (report)")),
            obj(2, 41.0, 0.0, None, Some("City: <<City>>")),
            obj(3, 41.0, 200.0, Some(TOTAL), None),
            obj(4, 61.0, 0.0, Some(NAME), None),
            obj(5, 61.0, 200.0, Some(AMOUNT), None),
            obj(6, 81.0, 0.0, None, Some("Grand")),
            obj(7, 81.0, 200.0, Some(TOTAL), None),
            obj(
                8,
                101.0,
                0.0,
                None,
                Some("Page {{PageNumber}} of {{PageCount}}"),
            ),
        ],
        ..Default::default()
    };
    Database {
        tables: vec![t],
        occurrences: vec![fmp12::graph::TableOccurrence {
            id: T,
            name: "People".into(),
            base_table: T,
            ..Default::default()
        }],
        relationships: vec![],
        layouts: vec![layout],
        themes: vec![],
        value_lists: vec![],
        locale: Default::default(),
    }
}

const PEOPLE: [(&str, &str, &str); 5] = [
    ("Ada", "Rome", "10"),
    ("Bob", "Paris", "20"),
    ("Cy", "Rome", "30"),
    ("Di", "Oslo", "40"),
    ("Ed", "Rome", ""),
];

fn sorted(db: &Database) -> Vec<usize> {
    let mut found: Vec<usize> = (0..db.tables[0].records.len()).collect();
    engine::sort(db, T, &mut found, &[(CITY, Order::Ascending)]);
    found
}

#[test]
fn parts_follow_preview_order() {
    let db = db(&PEOPLE);
    let found = sorted(&db);
    let sort = [(CITY, Order::Ascending)];
    let pages = paginate(&db, &db.layouts[0], &found, &sort, &Paper::letter());
    assert_eq!(pages.len(), 1);
    let kinds: Vec<(u64, Option<usize>)> = pages[0]
        .bands
        .iter()
        .map(|b| (db.layouts[0].parts[b.part].kind, b.record))
        .collect();
    // Oslo: Di; Paris: Bob; Rome: Ada, Cy, Ed.
    let s = part::LEADING_SUB_SUMMARY;
    let b = part::BODY;
    assert_eq!(
        kinds,
        vec![
            (part::HEADER, Some(3)),
            (s, Some(3)),
            (b, Some(3)),
            (s, Some(1)),
            (b, Some(1)),
            (s, Some(0)),
            (b, Some(0)),
            (b, Some(2)),
            (b, Some(4)),
            (part::TRAILING_GRAND_SUMMARY, Some(3)),
            (part::FOOTER, Some(4)),
        ]
    );
    // Footer sits at the bottom of the printable area.
    assert_eq!(pages[0].bands.last().unwrap().y, 720.0 - 20.0);
    // Unsorted: no sub-summaries.
    let pages = paginate(&db, &db.layouts[0], &[0, 1, 2], &[], &Paper::letter());
    assert_eq!(pages[0].bands.len(), 1 + 3 + 1 + 1);
}

fn page_count(pdf: &[u8]) -> usize {
    pdf.windows(12).filter(|w| w == b"/Type /Page ").count()
}

fn has(pdf: &[u8], s: &str) -> bool {
    pdf.windows(s.len()).any(|w| w == s.as_bytes())
}

#[test]
fn pdf_smoke() {
    let db = db(&PEOPLE);
    let found = sorted(&db);
    let (bytes, n) = pdf(
        &db,
        1,
        &found,
        &[(CITY, Order::Ascending)],
        &Paper::letter(),
    )
    .unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4\n"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    assert_eq!((n, page_count(&bytes)), (1, 1));
    for s in [
        "(People \\(report\\)) Tj",
        "(City: Rome) Tj",
        "(Ada) Tj",
        "(Grand) Tj",
        "(Page 1 of 1) Tj",
    ] {
        assert!(has(&bytes, s), "{s}");
    }
    // Sub-summary totals: Oslo 40, Paris 20, Rome 40; grand total 100.
    assert!(has(&bytes, "(20) Tj") && has(&bytes, "(40) Tj") && has(&bytes, "(100) Tj"));
}

#[test]
fn page_breaks_at_paper_size() {
    let names: Vec<String> = (0..100).map(|i| format!("P{i}")).collect();
    let rows: Vec<(&str, &str, &str)> = names.iter().map(|n| (n.as_str(), "X", "1")).collect();
    let db = db(&rows);
    let found: Vec<usize> = (0..100).collect();
    // 720 pt minus header 40 and footer 20 leaves 33 bodies a page;
    // 100 bodies and the grand summary need 4 pages.
    let (bytes, n) = pdf(&db, 1, &found, &[], &Paper::letter()).unwrap();
    assert_eq!((n, page_count(&bytes)), (4, 4));
    assert!(has(&bytes, "/Count 4"));
    assert!(has(&bytes, "(Page 4 of 4) Tj"));
    assert!(has(&bytes, "(P99) Tj"));
    // Landscape letter: 540 - 60 = 480, 24 a page: 5 pages.
    let (_, n) = pdf(&db, 1, &found, &[], &Paper::letter().landscape()).unwrap();
    assert_eq!(n, 5);
    let (_, n) = pdf(&db, 1, &found, &[], &Paper::a4().with_margins(0.0)).unwrap();
    assert_eq!(n, 3);
    // Empty found set: one page with header, grand summary and footer.
    let (bytes, n) = pdf(&db, 1, &[], &[], &Paper::letter()).unwrap();
    assert_eq!(n, 1);
    assert!(has(&bytes, "(Grand) Tj"));
}

#[test]
fn template_list_layout() {
    let dir =
        std::path::PathBuf::from(r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates");
    let path = dir.join("Assets").join("en.fmp12");
    if !path.exists() {
        return;
    }
    let mut db = Database::open(path).unwrap();
    let ti = db.tables.iter().position(|t| t.name == "Assets").unwrap();
    let tid = db.tables[ti].id;
    let name = db.tables[ti].field_by_name("Name").unwrap().id;
    let price = db.tables[ti].field_by_name("Price").unwrap().id;
    for (n, p) in [("Laptop", "1200"), ("Desk", "300.5")] {
        let r = db.new_record(tid).unwrap();
        db.tables[ti].records[r].values.insert(name, n.into());
        db.tables[ti].records[r].values.insert(price, p.into());
    }
    let layout = db
        .layouts
        .iter()
        .find(|l| l.name == "Asset List")
        .unwrap()
        .id;
    let (bytes, n) = pdf(&db, layout, &[0, 1], &[], &Paper::letter().landscape()).unwrap();
    assert_eq!((n, page_count(&bytes)), (1, 1));
    for s in [
        "(Laptop) Tj",
        "(Desk) Tj",
        "(Purchase Date) Tj",
        "(1500.5) Tj",
    ] {
        assert!(has(&bytes, s), "{s}");
    }
    // Themed: the body text is not plain black everywhere.
    assert!(has(&bytes, " rg "));
}
