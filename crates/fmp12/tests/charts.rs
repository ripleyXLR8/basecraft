//! Charts in `fixtures/charts.fmp12`: a file made by `basecraft-cli` (Sales
//! and Regions tables) whose 14 charts were pasted into the Sales layout in
//! FileMaker Pro 26 (see `docs/FORMAT.md` §10, "Charts").
#![allow(clippy::unwrap_used)]

use fmp12::charts::{Kind, Source, charts};

#[test]
fn charts_of_the_fixture() {
    let f = fmp12::File::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/charts.fmp12"
    ))
    .unwrap();
    let c = charts(&f).unwrap();
    let kinds: Vec<Kind> = c.values().map(|c| c.kind).collect();
    use Kind::*;
    assert_eq!(
        kinds,
        [
            Column,
            Column,
            StackedColumn,
            PositiveNegative,
            Bar,
            StackedBar,
            Line,
            Area,
            Pie,
            Scatter,
            Bubble,
            Column,
            Column,
            Column
        ]
    );
    let get = |o: u64| &c[&(1, o)];
    struct NoNames;
    impl fmp12::calc::Names for NoNames {
        fn field(&self, _: u32, _: u32) -> Option<String> {
            None
        }
        fn occurrence(&self, _: u32) -> Option<String> {
            None
        }
    }
    let text = |t: &Option<Vec<u8>>| {
        fmp12::calc::decompile(t.as_deref().unwrap(), 0, &NoNames).unwrap_or_default()
    };
    // Two series, legend and data points, found set.
    let two = get(2);
    assert_eq!(two.y.len(), 2);
    assert!(two.legend && two.data_points);
    assert_eq!(two.source, Source::FoundSet);
    assert_eq!(text(&two.title), "\"Two series\"");
    assert_eq!(text(&two.y[1].name), "\"Cost\"");
    // Bubble: two series and a size.
    assert_eq!(get(11).y.len(), 2);
    assert!(get(11).size.is_some());
    // Delimited data of the current record, axis titles, schemes.
    let earth = get(12);
    assert_eq!(earth.source, Source::CurrentRecord);
    assert_eq!(text(&earth.x_title), "\"Index\"");
    assert_eq!((earth.scheme, get(13).scheme), (3, 0));
    // Related records of Regions (occurrence 130).
    assert_eq!(get(14).source, Source::Related(130));
    assert!(!get(1).legend && !get(1).data_points);
}
