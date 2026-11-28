//! Makes `assets/blank.fmp12` from a new file created with FileMaker Pro
//! (File › New Solution, then opened and closed once so that it holds its
//! data formats, `[2]` key 134): removes the default fields of its table,
//! replaces the OS user name kept in object metadata (key 31873), and
//! replaces FileMaker's theme with BaseCraft's own plain theme (white
//! layout and body, light grey header, footer and navigation parts, edit
//! boxes with a thin grey border, black 12 pt text; FORMAT.md §12).
//! `cargo run -p fmp12 --example mkblank -- <in.fmp12> <out.fmp12>`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::schema_edit::Editor;
use fmp12::write::{Entry, Item};

/// Protobuf writer for the few wire types the styles use.
fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}
fn key(f: u64, wire: u64, out: &mut Vec<u8>) {
    varint(f << 3 | wire, out);
}
fn uint(f: u64, v: u64) -> Vec<u8> {
    let mut o = Vec::new();
    key(f, 0, &mut o);
    varint(v, &mut o);
    o
}
fn msg(f: u64, body: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    key(f, 2, &mut o);
    varint(body.len() as u64, &mut o);
    o.extend_from_slice(body);
    o
}
fn f32f(f: u64, v: f32) -> Vec<u8> {
    let mut o = Vec::new();
    key(f, 5, &mut o);
    o.extend(v.to_le_bytes());
    o
}
fn f64f(f: u64, v: f64) -> Vec<u8> {
    let mut o = Vec::new();
    key(f, 1, &mut o);
    o.extend(v.to_le_bytes());
    o
}

/// Property {1: id, 5: colour}.
fn colour(id: u64, r: f32, g: f32, b: f32) -> Vec<u8> {
    let c = [f32f(1, r), f32f(2, g), f32f(3, b), f32f(4, 1.0)].concat();
    [uint(1, id), msg(5, &c)].concat()
}
/// Property {1: id, 3: enum}.
fn enumv(id: u64, v: u64) -> Vec<u8> {
    [uint(1, id), uint(3, v)].concat()
}
/// Property {1: id, 6: length in points}.
fn pt(id: u64, v: f64) -> Vec<u8> {
    [
        uint(1, id),
        msg(6, &[f64f(1, v), uint(2, 6), uint(3, 0)].concat()),
    ]
    .concat()
}

/// One rule (element, :normal) with `props`.
fn rule(element: u64, props: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = props.iter().flat_map(|p| msg(1, p)).collect();
    msg(1, &[uint(1, element), uint(2, 1), msg(3, &body)].concat())
}

/// Default style of `object_type`: one rule (.self, :normal) with `props`.
fn style(object_type: u64, props: &[Vec<u8>]) -> Vec<u8> {
    styled(object_type, &[rule(1, props)])
}

/// Default style of `object_type` with several rules.
fn styled(object_type: u64, rules: &[Vec<u8>]) -> Vec<u8> {
    msg(1, &[uint(1, object_type), msg(2, &rules.concat())].concat())
}

/// Property {1: id, 6: length with `unit`}.
fn len(id: u64, v: f64, unit: u64) -> Vec<u8> {
    [
        uint(1, id),
        msg(6, &[f64f(1, v), uint(2, unit), uint(3, 0)].concat()),
    ]
    .concat()
}

/// Compiled styles of the plain theme.
fn plain_styles() -> Vec<u8> {
    let (w, l) = (1.0, 0.94);
    let border: Vec<Vec<u8>> = (20..24)
        .map(|i| colour(i, 0.75, 0.75, 0.75))
        .chain((24..28).map(|i| enumv(i, 5)))
        .chain((28..32).map(|i| pt(i, 1.0)))
        .collect();
    // Font (first font of `[25]`), style, size, colour, line height, left.
    let text = vec![
        enumv(64, 0),
        enumv(67, 1),
        pt(69, 12.0),
        colour(70, 0.0, 0.0, 0.0),
        len(72, 1.0, 9),
        enumv(74, 1),
    ];
    // `.text` box: full height (width too for buttons), border-box.
    let text_box = rule(3, &[len(110, 100.0, 1), enumv(123, 1)]);
    let button_box = rule(3, &[len(109, 100.0, 1), len(110, 100.0, 1), enumv(123, 1)]);
    let button = [
        vec![colour(7, l, l, l)],
        text.clone(),
        vec![enumv(74, 2), enumv(611, 1)],
    ]
    .concat();
    [
        style(20, &[colour(7, w, w, w)]),
        style(34, &[colour(7, w, w, w)]),
        style(32, &[colour(7, l, l, l)]),
        style(38, &[colour(7, l, l, l)]),
        style(40, &[colour(7, l, l, l)]),
        styled(
            61,
            &[
                rule(
                    1,
                    &[vec![colour(7, w, w, w)], border, text.clone()].concat(),
                ),
                text_box.clone(),
            ],
        ),
        styled(70, &[rule(1, &text), text_box]),
        styled(80, &[rule(1, &button), button_box]),
    ]
    .concat()
}

/// The theme node `[6].[5].[1]` and its catalog entry, replaced.
fn replace_theme(es: &mut Vec<Entry>) {
    let theme = vec![vec![6], vec![5], vec![1]];
    let p = |x: &[u8]| x.iter().map(|b| vec![*b]).collect::<Vec<_>>();
    es.retain(|e| {
        let under = e.path.starts_with(&theme) && e.path.len() > 3;
        let named = e.path == theme
            && matches!(e.item.first(), Some(0x02..=0x07))
            && matches!(e.item.get(1), Some(10 | 13 | 16 | 22));
        let index = e.path == p(&[6, 1, 1]) && e.item != [0x80];
        !(under || named || index)
    });
    let kv = |key: u32, data: Vec<u8>| Entry {
        path: theme.clone(),
        item: fmp12::write::encode(&Item::Kv { key, data }),
    };
    let id = "org.basecraft.theme.plain";
    es.push(kv(10, fmp12::save::encode_text("BaseCraft")));
    es.push(kv(13, fmp12::save::encode_text("BaseCraft")));
    es.push(kv(16, fmp12::save::encode_text(id)));
    es.push(kv(22, plain_styles()));
    // Theme catalog: index of theme ids.
    if let Some(key) = fmp12::collate::name_key(id) {
        es.push(Entry {
            path: p(&[6, 1, 1]),
            item: fmp12::write::encode(&Item::KvLong {
                key,
                data: vec![1, 1],
            }),
        });
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = fmp12::File::open(&a[0]).unwrap();
    // Metadata: user name → "BaseCraft".
    let mut es = fmp12::write::entries(&file).unwrap();
    for e in es.iter_mut() {
        if let Ok(c) = fmp12::chunk::parse(&e.item)
            && let [fmp12::Chunk::Kv { key: 31873, .. }] = c.as_slice()
        {
            e.item = fmp12::write::encode(&fmp12::write::Item::Kv {
                key: 31873,
                data: fmp12::save::encode_text("BaseCraft"),
            });
        }
    }
    replace_theme(&mut es);
    fmp12::write::sort(&mut es);
    let bytes = fmp12::write::build(&file.bytes()[..fmp12::SECTOR], &es);
    let file = fmp12::File::from_bytes(bytes).unwrap();
    let mut ed = Editor::open(&file).unwrap();
    let names = ed.names();
    // Only FileMaker's new table; other files just get the theme.
    for t in names.tables.iter().filter(|t| t.name == "Blank") {
        for f in t.fields.keys() {
            ed.delete_field(t.id, *f).unwrap();
        }
    }
    std::fs::write(&a[1], ed.finish()).unwrap();
}
