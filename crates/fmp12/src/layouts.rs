//! Layouts (`[4]`). Most layout data is protobuf ([`crate::pb`]):
//!
//! * `[4].[5].[L]` key 16 = name, key 2 = header message
//!   (2 = table occurrence, 8 = width);
//! * `[4].[5].[L].[3].[P]` key 2 = part message
//!   (1 = kind, 4 = top, 5 = height, 10 = style);
//! * object list message: key 7 of the layout node, or when longer than
//!   1000 bytes, split over keys 1…n of `[4].[5].[L].[7]` (key 0 = total
//!   length). Repeated field 2 = object
//!   (1 = kind, 2 = id, 3 = options, 4 = bounds, 5 = style, 8 = field);
//! * `[4].[5].[L]` key 11 = theme number (`01 n`), see [`crate::theme`];
//! * `[4].[5].[L].[5].[obj]` key 1 = static text of a text object.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, pb::Msg, text};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    /// 1 Header, 2 Footer?, 3 Leading sub-summary, 4 Body, 12 Top navigation…
    pub kind: u64,
    pub top: f64,
    pub height: f64,
    pub style: String,
    /// Raw style message (field 10), see [`crate::theme::ObjectStyle`].
    pub style_raw: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bounds {
    pub top: f64,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutObject {
    pub id: u64,
    /// 1 Field, 2 Text, 10 Button, 18 Button bar… (XML `kind`).
    pub kind: u64,
    pub options: u64,
    pub bounds: Bounds,
    pub style: String,
    /// Raw style message (field 5), see [`crate::theme::ObjectStyle`].
    pub style_raw: Vec<u8>,
    /// (table occurrence id as in XML, field id, repetition)
    pub field: Option<(u64, u64, u64)>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub id: u32,
    pub name: String,
    /// Table occurrence id as in XML (`0x104000 | id`), 0 for folders.
    pub table_occurrence: u64,
    pub width: f64,
    /// Theme number (`[6].[5].[n]`), from layout key 11 (`01 n`); 0 if none.
    pub theme: u32,
    /// Raw header style message (field 5) and local CSS (field 7),
    /// see [`crate::theme::ObjectStyle::of_layout`].
    pub style_raw: Vec<u8>,
    pub css_raw: Vec<u8>,
    pub parts: Vec<Part>,
    pub objects: Vec<LayoutObject>,
}

#[derive(Default)]
struct Raw {
    name: String,
    header: Vec<u8>,
    theme: u32,
    parts: BTreeMap<u32, Vec<u8>>,
    objects: BTreeMap<u32, Vec<u8>>,
    texts: BTreeMap<u32, String>,
}

fn style_name(m: &Msg) -> String {
    m.str(2).unwrap_or_default().to_string()
}

fn object(m: &Msg, texts: &BTreeMap<u32, String>) -> LayoutObject {
    let b = m.msg(4).unwrap_or_default();
    let id = m.uint(2).unwrap_or(0);
    LayoutObject {
        id,
        kind: m.uint(1).unwrap_or(0),
        options: m.uint(3).unwrap_or(0),
        bounds: Bounds {
            top: b.f64(1).unwrap_or(0.0),
            left: b.f64(2).unwrap_or(0.0),
            bottom: b.f64(3).unwrap_or(0.0),
            right: b.f64(4).unwrap_or(0.0),
        },
        style: m.msg(5).map(|s| style_name(&s)).unwrap_or_default(),
        style_raw: m.bytes(5).unwrap_or_default().to_vec(),
        field: m.msg(8).and_then(|f| f.msg(2)).map(|r| {
            (
                r.uint(1).unwrap_or(0),
                r.uint(2).unwrap_or(0),
                r.uint(3).unwrap_or(1),
            )
        }),
        text: u32::try_from(id).ok().and_then(|i| texts.get(&i).cloned()),
    }
}

pub fn layouts(file: &File) -> Result<Vec<Layout>, Error> {
    let mut raw: BTreeMap<u32, Raw> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        let Chunk::Kv { key, data } = c else { return };
        match p.as_slice() {
            [4, 5, l] => {
                let r = raw.entry(*l).or_default();
                match key {
                    16 => r.name = text(data),
                    2 => r.header = data.to_vec(),
                    // `01 n`: byte count, then the theme number.
                    11 => r.theme = data[1..].iter().fold(0, |a, b| (a << 8) | *b as u32),
                    // Short object lists are stored inline instead of in [7].
                    7 => {
                        r.objects.insert(1, data.to_vec());
                    }
                    _ => {}
                }
            }
            [4, 5, l, 3, part] if *key == 2 => {
                raw.entry(*l)
                    .or_default()
                    .parts
                    .insert(*part, data.to_vec());
            }
            [4, 5, l, 7] if *key >= 1 => {
                raw.entry(*l)
                    .or_default()
                    .objects
                    .insert(*key, data.to_vec());
            }
            [4, 5, l, 5, obj] if *key == 1 => {
                raw.entry(*l).or_default().texts.insert(*obj, text(data));
            }
            _ => {}
        }
    })?;
    Ok(raw
        .into_iter()
        .map(|(id, r)| {
            let header = Msg::parse(&r.header).unwrap_or_default();
            let parts = r
                .parts
                .values()
                .filter_map(|b| Msg::parse(b))
                .map(|m| Part {
                    kind: m.uint(1).unwrap_or(0),
                    top: m.f64(4).unwrap_or(0.0),
                    height: m.f64(5).unwrap_or(0.0),
                    style: m.msg(10).map(|s| style_name(&s)).unwrap_or_default(),
                    style_raw: m.bytes(10).unwrap_or_default().to_vec(),
                })
                .collect();
            let blob: Vec<u8> = r.objects.into_values().flatten().collect();
            let objects = Msg::parse(&blob)
                .map(|m| m.msgs(2).map(|o| object(&o, &r.texts)).collect())
                .unwrap_or_default();
            Layout {
                id,
                name: r.name,
                table_occurrence: header.uint(2).unwrap_or(0),
                width: header.f64(8).unwrap_or(0.0),
                theme: r.theme,
                style_raw: header.bytes(5).unwrap_or_default().to_vec(),
                css_raw: header.bytes(7).unwrap_or_default().to_vec(),
                parts,
                objects,
            }
        })
        .collect())
}
