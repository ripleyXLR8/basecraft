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

/// Chart Setup edits written by `save_layouts` read back; the other charts
/// keep their bytes.
#[test]
fn chart_edits_are_saved() {
    let f = fmp12::File::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/charts.fmp12"
    ))
    .unwrap();
    let mut layouts = fmp12::layouts::layouts(&f).unwrap();
    let before = charts(&f).unwrap();
    let l = layouts.iter_mut().find(|l| l.id == 1).unwrap();
    let o = l.objects.iter_mut().find(|o| o.id == 2).unwrap();
    let c = o.chart.as_mut().unwrap();
    assert_eq!(**c, before[&(1, 2)]);
    let title = fmp12::save::text_literal_tokens("New title");
    c.title = Some(title.clone());
    c.y[1].name = Some(fmp12::save::text_literal_tokens("Costs"));
    c.x_title = Some(fmp12::save::text_literal_tokens("Months"));
    c.kind = Kind::Line;
    c.legend = false;
    c.source = Source::Related(130);
    let out = fmp12::save::save_layouts(&f, &layouts).unwrap();
    // `BASECRAFT_CHART_DEMO=<out.fmp12>` keeps the file (checked with
    // `tools/oracle/fms_check.sh`).
    if let Ok(p) = std::env::var("BASECRAFT_CHART_DEMO") {
        std::fs::write(p, &out).unwrap();
    }
    let g = fmp12::File::from_bytes(out).unwrap();
    let after = charts(&g).unwrap();
    let c = &after[&(1, 2)];
    assert_eq!(c.title.as_ref(), Some(&title));
    assert_eq!(c.y[1].name, Some(fmp12::save::text_literal_tokens("Costs")));
    assert!(c.x_title.is_some());
    assert_eq!(
        (c.kind, c.legend, c.source),
        (Kind::Line, false, Source::Related(130))
    );
    assert_eq!(c.y[0], before[&(1, 2)].y[0]);
    assert_eq!(c.record.len(), before[&(1, 2)].record.len() + 2);
    for (k, v) in &before {
        if *k != (1, 2) {
            assert_eq!(&after[k], v);
        }
    }
    // The re-read layout carries the same chart.
    let again = fmp12::layouts::layouts(&g).unwrap();
    let o = again[0].objects.iter().find(|o| o.id == 2).unwrap();
    assert_eq!(o.chart.as_deref(), Some(c));
}

/// A chart's series lists grow and shrink as FileMaker writes them: the
/// two-series column chart cut to one series has the lists of the
/// one-series column chart, and grown back it has its own again.
#[test]
fn series_lists_follow_the_series() {
    let f = fmp12::File::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/charts.fmp12"
    ))
    .unwrap();
    let c = charts(&f).unwrap();
    let two = &c[&(1, 1)];
    let one = &c[&(1, 12)];
    assert_eq!((two.y.len(), one.y.len()), (2, 1));
    let lists = 296..383;
    let mut cut = two.clone();
    cut.y.truncate(1);
    let b = cut.encode_record();
    assert_eq!(b.len(), one.record.len());
    assert_eq!(b[lists.clone()], one.record[lists]);
    cut.y.push(two.y[1].clone());
    assert_eq!(cut.encode_record(), two.record);
}
