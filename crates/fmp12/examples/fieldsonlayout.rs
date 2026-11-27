//! Puts every field of a table on the body of the table's layout, one
//! after the other, with the style of an existing edit box of the file, so
//! that FileMaker's Table view shows them (`tools/oracle/summary_demo.sh`).
//! `cargo run -p fmp12 --example fieldsonlayout -- <in.fmp12> <Table> <out.fmp12>`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = fmp12::File::open(&a[1]).unwrap();
    let tables = fmp12::schema::tables(&f).unwrap();
    let t = tables.iter().find(|t| t.name == a[2]).unwrap();
    let mut ls = fmp12::layouts::layouts(&f).unwrap();
    // A field object built on an existing edit box keeps its data formats:
    // without them FileMaker shows no numbers.
    let style = ls
        .iter()
        .flat_map(|l| l.objects.iter())
        .find(|o| o.kind == 1 && fmp12::theme::ObjectStyle::parse(&o.style_raw).object_type == 61)
        .map(|o| o.style_raw.clone())
        .expect("an edit box in the file");
    let l = ls.iter_mut().find(|l| l.name == t.name).unwrap();
    let body = l.parts.iter().find(|p| p.kind == 4).unwrap().clone();
    let first = l.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
    let mut x = 10.0;
    for (id, fid) in (first..).zip(t.fields.keys()) {
        l.objects.insert(
            0,
            fmp12::layouts::LayoutObject {
                id,
                kind: 1,
                bounds: fmp12::layouts::Bounds {
                    top: body.top + 4.0,
                    left: x,
                    bottom: body.top + 24.0,
                    right: x + 110.0,
                },
                style_raw: style.clone(),
                field: Some((l.table_occurrence, *fid as u64, 1)),
                ..Default::default()
            },
        );
        x += 114.0;
    }
    l.width = l.width.max(x + 10.0);
    std::fs::write(&a[3], fmp12::save::save_layouts(&f, &ls).unwrap()).unwrap();
}
