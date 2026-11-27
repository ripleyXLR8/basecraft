//! Themes (`[6]`) and object styles. See `docs/FORMAT.md` §12.
//!
//! A theme compiles FileMaker's CSS into protobuf. Every style is a list of
//! rules, one per (element, state), each holding `{1: property id, value}`
//! entries. The same rule list is stored on each layout object as its local
//! style, so one decoder serves both.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, layouts::Layout, pb::Msg, text};

/// Elements (CSS pseudo-elements such as `.self`, `.text`).
pub const SELF: u32 = 1;
pub const INNER_BORDER: u32 = 2;
pub const TEXT: u32 = 3;
pub const ICON: u32 = 4;

/// State `:normal`.
pub const NORMAL: u32 = 1;

/// Property ids used by most renderers. Full table: [`css_name`].
pub mod prop {
    pub const BACKGROUND_COLOR: u32 = 7;
    /// Border colour of the top side; right, bottom and left follow (21–23).
    pub const BORDER_COLOR: u32 = 20;
    /// Border style of the top side (`Enum`: 1 none, 5 solid); then 25–27.
    pub const BORDER_STYLE: u32 = 24;
    /// Border width of the top side; then 29–31.
    pub const BORDER_WIDTH: u32 = 28;
    /// Top-left corner radius (`Pair`); then top-right, bottom-right, bottom-left.
    pub const BORDER_RADIUS: u32 = 32;
    /// `Enum`: index into the font catalog, font id = 128 + value ([`super::fonts`]).
    pub const FONT_FAMILY: u32 = 64;
    /// `Enum`: 2 bold.
    pub const FONT_WEIGHT: u32 = 65;
    /// `Enum`: 1 normal.
    pub const FONT_STYLE: u32 = 67;
    pub const FONT_SIZE: u32 = 69;
    pub const COLOR: u32 = 70;
    pub const LINE_HEIGHT: u32 = 72;
    /// `Enum`: 1 left, 2 center, 3 right.
    pub const TEXT_ALIGN: u32 = 74;
    /// `Enum`: 1 none, 3 uppercase.
    pub const TEXT_TRANSFORM: u32 = 75;
    /// Padding top; then right, bottom, left (102–104).
    pub const PADDING: u32 = 101;
    /// Margin top; then right, bottom, left (106–108).
    pub const MARGIN: u32 = 105;
    /// `Enum`: 0 top, 1 center, 2 bottom.
    pub const VERTICAL_ALIGN: u32 = 611;
}

/// CSS name of a property id, as found in FileMaker's theme files.
pub fn css_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "background-image",
        7 => "background-color",
        20 => "border-top-color",
        21 => "border-right-color",
        22 => "border-bottom-color",
        23 => "border-left-color",
        24 => "border-top-style",
        25 => "border-right-style",
        26 => "border-bottom-style",
        27 => "border-left-style",
        28 => "border-top-width",
        29 => "border-right-width",
        30 => "border-bottom-width",
        31 => "border-left-width",
        32 => "border-top-left-radius",
        33 => "border-top-right-radius",
        34 => "border-bottom-right-radius",
        35 => "border-bottom-left-radius",
        36 => "border-image-source",
        64 => "font-family",
        65 => "font-weight",
        67 => "font-style",
        69 => "font-size",
        70 => "color",
        72 => "line-height",
        74 => "text-align",
        75 => "text-transform",
        101 => "padding-top",
        102 => "padding-right",
        103 => "padding-bottom",
        104 => "padding-left",
        105 => "margin-top",
        106 => "margin-right",
        107 => "margin-bottom",
        108 => "margin-left",
        109 => "width",
        110 => "height",
        117 => "top",
        118 => "right",
        119 => "bottom",
        120 => "left",
        121 => "position",
        122 => "box-shadow",
        123 => "box-sizing",
        513 => "-fm-icon",
        514 => "-fm-icon-color",
        515 => "-fm-tab-spacing",
        610 => "-fm-portal-alt-background",
        611 => "-fm-text-vertical-align",
        616 => "-fm-body-alt-background",
        619 => "-fm-use-body-current-row-style",
        620 => "-fm-use-portal-current-row-style",
        621 => "-fm-icon-padding",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    /// Unit 0, seen on zero lengths.
    None,
    Percent,
    Pt,
    Em,
    /// Multiple of the font's line height (`line-height: 1line`).
    Line,
    Other(u64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    pub value: f64,
    pub unit: Unit,
}

impl Length {
    /// The length in points, given the font size for `em` and the reference
    /// size for `%`.
    pub fn pt(&self, font_size: f64, percent_of: f64) -> f64 {
        match self.unit {
            Unit::Em | Unit::Line => self.value * font_size,
            Unit::Percent => self.value * percent_of / 100.0,
            _ => self.value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `#rrggbb` (alpha dropped).
    pub fn hex(&self) -> String {
        let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Wire field 3: keyword or integer (see the [`prop`] docs for values).
    Enum(i64),
    /// Wire field 5.
    Color(Rgba),
    /// Wire field 6.
    Length(Length),
    /// Wire field 13: two lengths (corner radius: horizontal, vertical).
    Pair(Length, Length),
    /// Wire field 19 (`box-shadow`).
    Shadow {
        inset: bool,
        color: Rgba,
        x: Length,
        y: Length,
        blur: Length,
        spread: Length,
    },
    /// `none` (an image or shadow message whose field 1 is 0).
    None,
    /// Any other wire field, undecoded.
    Raw(u32, Vec<u8>),
}

/// Properties of one rule, by property id.
pub type Props = BTreeMap<u32, Value>;
/// Rules of one style, keyed by (element, state).
pub type Rules = BTreeMap<(u32, u32), Props>;

fn color(m: &Msg) -> Rgba {
    let f = |i| m.f64(i).unwrap_or(0.0) as f32;
    Rgba {
        r: f(1),
        g: f(2),
        b: f(3),
        a: f(4),
    }
}

fn length(m: &Msg) -> Length {
    Length {
        value: m.f64(1).unwrap_or(0.0),
        unit: match m.uint(2).unwrap_or(0) {
            0 => Unit::None,
            1 => Unit::Percent,
            6 => Unit::Pt,
            7 => Unit::Em,
            9 => Unit::Line,
            u => Unit::Other(u),
        },
    }
}

fn value(field: u32, b: &[u8]) -> Value {
    let m = Msg::parse(b).unwrap_or_default();
    let len = |i| length(&m.msg(i).unwrap_or_default());
    match field {
        5 => Value::Color(color(&m)),
        6 => Value::Length(length(&m)),
        13 => Value::Pair(len(1), len(2)),
        12 | 18 | 19 if m.uint(1) == Some(0) => Value::None,
        19 => Value::Shadow {
            inset: m.uint(2) == Some(1),
            color: color(&m.msg(3).unwrap_or_default()),
            x: len(4),
            y: len(5),
            blur: len(6),
            spread: len(7),
        },
        _ => Value::Raw(field, b.to_vec()),
    }
}

/// Decodes a rule list: repeated `1: {1: element, 2: state, 3: {repeated 1: property}}`.
pub fn rules(b: &[u8]) -> Rules {
    let mut out = Rules::new();
    for r in Msg::parse(b).unwrap_or_default().msgs(1) {
        let key = (r.uint(1).unwrap_or(0) as u32, r.uint(2).unwrap_or(0) as u32);
        let props = out.entry(key).or_default();
        for p in r.msg(3).unwrap_or_default().msgs(1) {
            let Some(id) = p.uint(1) else { continue };
            let v = p.0.iter().find(|(f, _)| *f != 1).map(|(f, v)| match v {
                crate::pb::Value::Varint(x) => Value::Enum(*x as i64),
                crate::pb::Value::Bytes(b) => value(*f, b),
                _ => Value::Raw(*f, Vec::new()),
            });
            if let Some(v) = v {
                props.insert(id as u32, v);
            }
        }
    }
    out
}

/// The style message of a layout object (field 5), part (field 10) or
/// layout header (field 5): `{1: local rules, 2: named style, 3: object type}`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectStyle {
    pub object_type: u32,
    /// Named style, empty for the theme default.
    pub name: String,
    pub local: Rules,
}

impl ObjectStyle {
    pub fn parse(b: &[u8]) -> Self {
        let m = Msg::parse(b).unwrap_or_default();
        ObjectStyle {
            object_type: m.uint(3).unwrap_or(0) as u32,
            name: m.str(2).unwrap_or_default().to_string(),
            local: m.bytes(1).map(rules).unwrap_or_default(),
        }
    }

    /// Style of a layout's background: header field 5, local rules in field 7.
    pub fn of_layout(l: &Layout) -> Self {
        ObjectStyle {
            local: ObjectStyle::parse(&l.css_raw).local,
            ..ObjectStyle::parse(&l.style_raw)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Theme {
    /// Catalog id, referenced by layouts ([`crate::layouts::Layout::theme`]).
    pub number: u32,
    /// Display name, e.g. "Universal Touch".
    pub name: String,
    /// e.g. "com.filemaker.theme.universal_touch".
    pub id: String,
    /// Keyed by (object type, named style); the empty name is the default style.
    pub styles: BTreeMap<(u32, String), Rules>,
}

impl Theme {
    /// Decodes the compiled style message: repeated 1 = default style
    /// `{1: type, 2: rules}`, repeated 2 = named style `{1: name, 2: rules, 3: type}`.
    pub fn parse_styles(b: &[u8]) -> BTreeMap<(u32, String), Rules> {
        let m = Msg::parse(b).unwrap_or_default();
        let mut out = BTreeMap::new();
        for e in m.msgs(1) {
            let r = e.bytes(2).map(rules).unwrap_or_default();
            out.insert((e.uint(1).unwrap_or(0) as u32, String::new()), r);
        }
        for e in m.msgs(2) {
            let r = e.bytes(2).map(rules).unwrap_or_default();
            let name = e.str(1).unwrap_or_default().to_string();
            out.insert((e.uint(3).unwrap_or(0) as u32, name), r);
        }
        out
    }

    fn default_rules(&self, ty: u32) -> Option<&Rules> {
        let get = |t| self.styles.get(&(t, String::new()));
        // ponytail: sub-summary variants 101, 201… fall back to 100, 200 when
        // the theme lacks them; exact meaning of the variants is unknown.
        get(ty).or_else(|| (ty >= 100).then(|| get(ty - ty % 100)).flatten())
    }

    fn named_rules(&self, ty: u32, name: &str) -> Option<&Rules> {
        if name.is_empty() {
            return None;
        }
        // Part and shape styles are stored under a generic type (30, 54).
        self.styles.get(&(ty, name.to_string())).or_else(|| {
            self.styles
                .iter()
                .find(|((_, n), _)| n == name)
                .map(|(_, r)| r)
        })
    }

    /// Effective properties of `element` in `state`: theme default for the
    /// object type, then the named style, then the object's local style.
    /// Within each layer the `:normal` rule applies first, then `state`.
    pub fn resolve(&self, style: &ObjectStyle, element: u32, state: u32) -> Props {
        let mut out = Props::new();
        let layers = [
            self.default_rules(style.object_type),
            self.named_rules(style.object_type, &style.name),
            Some(&style.local),
        ];
        for rules in layers.into_iter().flatten() {
            for s in [NORMAL, state] {
                if let Some(p) = rules.get(&(element, s)) {
                    out.extend(p.iter().map(|(k, v)| (*k, v.clone())));
                }
                if state == NORMAL {
                    break;
                }
            }
        }
        out
    }
}

/// Typed lookups on resolved properties.
pub trait PropsExt {
    fn color(&self, id: u32) -> Option<Rgba>;
    fn length(&self, id: u32) -> Option<Length>;
    fn enum_value(&self, id: u32) -> Option<i64>;
}

impl PropsExt for Props {
    fn color(&self, id: u32) -> Option<Rgba> {
        match self.get(&id)? {
            Value::Color(c) => Some(*c),
            _ => None,
        }
    }
    fn length(&self, id: u32) -> Option<Length> {
        match self.get(&id)? {
            Value::Length(l) => Some(*l),
            _ => None,
        }
    }
    fn enum_value(&self, id: u32) -> Option<i64> {
        match self.get(&id)? {
            Value::Enum(v) => Some(*v),
            _ => None,
        }
    }
}

/// Themes of the file: `[6].[5].[n]` key 10 = name, 16 = id, 22 (or long
/// value `[22]`) = compiled styles.
pub fn themes(file: &File) -> Result<Vec<Theme>, Error> {
    let mut raw: BTreeMap<u32, (Theme, BTreeMap<u32, Vec<u8>>)> = BTreeMap::new();
    file.walk(|path, c| {
        let (Some(p), Chunk::Kv { key, data }) = (int_path(path), c) else {
            return;
        };
        match p.as_slice() {
            [6, 5, n] => {
                let (t, parts) = raw.entry(*n).or_default();
                match key {
                    10 => t.name = text(data),
                    16 => t.id = text(data),
                    22 => {
                        parts.insert(1, data.to_vec());
                    }
                    _ => {}
                }
            }
            [6, 5, n, 22] if *key >= 1 => {
                raw.entry(*n).or_default().1.insert(*key, data.to_vec());
            }
            _ => {}
        }
    })?;
    Ok(raw
        .into_iter()
        .map(|(number, (t, parts))| {
            let blob: Vec<u8> = parts.into_values().flatten().collect();
            Theme {
                number,
                styles: Theme::parse_styles(&blob),
                ..t
            }
        })
        .collect())
}

/// Font catalog `[25].[5].[id]` key 16 = family name (ids start at 128).
pub fn fonts(file: &File) -> Result<BTreeMap<u32, String>, Error> {
    let mut out = BTreeMap::new();
    file.walk(|path, c| {
        if let (Some([25, 5, id]), Chunk::Kv { key: 16, data }) = (int_path(path).as_deref(), c) {
            out.insert(*id, text(data));
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Protobuf length-delimited field.
    fn ld(field: u32, body: &[u8]) -> Vec<u8> {
        let tag = field << 3 | 2;
        let mut v = if tag < 0x80 {
            vec![tag as u8]
        } else {
            vec![(tag as u8) | 0x80, (tag >> 7) as u8]
        };
        v.push(body.len() as u8);
        v.extend_from_slice(body);
        v
    }
    fn vi(field: u32, x: u8) -> Vec<u8> {
        vec![(field << 3) as u8, x]
    }
    fn rgba(c: [f32; 4]) -> Vec<u8> {
        let mut v = Vec::new();
        for (i, x) in c.iter().enumerate() {
            v.push(((i as u8 + 1) << 3) | 5);
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }
    fn len(x: f64, unit: u8) -> Vec<u8> {
        let mut v = vec![0x09];
        v.extend_from_slice(&x.to_le_bytes());
        v.extend(vi(2, unit));
        v
    }
    fn prop(id: u8, field: u32, body: &[u8]) -> Vec<u8> {
        let mut p = vi(1, id);
        p.extend(if field == 3 {
            vi(3, body[0])
        } else {
            ld(field, body)
        });
        ld(1, &p)
    }
    fn rule(element: u8, state: u8, props: &[Vec<u8>]) -> Vec<u8> {
        let mut r = vi(1, element);
        r.extend(vi(2, state));
        r.extend(ld(3, &props.concat()));
        ld(1, &r)
    }

    #[test]
    fn decodes_values() {
        let r = rules(
            &[
                rule(
                    1,
                    1,
                    &[
                        prop(70, 5, &rgba([0.0, 0.478, 1.0, 1.0])),
                        prop(69, 6, &len(16.0, 6)),
                        prop(74, 3, &[2]),
                        prop(
                            32,
                            13,
                            &[ld(1, &len(50.0, 1)), ld(2, &len(50.0, 1))].concat(),
                        ),
                        prop(1, 18, &vi(1, 0)),
                    ],
                ),
                rule(3, 4, &[prop(101, 6, &len(0.75, 7))]),
            ]
            .concat(),
        );
        let p = &r[&(1, 1)];
        assert_eq!(p.color(prop::COLOR).unwrap().hex(), "#007aff");
        assert_eq!(
            p.length(prop::FONT_SIZE),
            Some(Length {
                value: 16.0,
                unit: Unit::Pt
            })
        );
        assert_eq!(p.enum_value(prop::TEXT_ALIGN), Some(2));
        let pct = Length {
            value: 50.0,
            unit: Unit::Percent,
        };
        assert_eq!(p[&32], Value::Pair(pct, pct));
        assert_eq!(p[&1], Value::None);
        assert_eq!(r[&(3, 4)].length(prop::PADDING).unwrap().unit, Unit::Em);
    }

    #[test]
    fn resolves_layers() {
        let white = rgba([1.0; 4]);
        let black = rgba([0.0, 0.0, 0.0, 1.0]);
        // default: white background, 12pt; pressed: black background
        let default = [
            rule(1, 1, &[prop(7, 5, &white), prop(69, 6, &len(12.0, 6))]),
            rule(1, 3, &[prop(7, 5, &black)]),
        ]
        .concat();
        // named "big": 20pt
        let named = rule(1, 1, &[prop(69, 6, &len(20.0, 6))]);
        let styles = [
            ld(1, &[vi(1, 80), ld(2, &default)].concat()),
            ld(2, &[ld(1, b"big"), ld(2, &named), vi(3, 80)].concat()),
        ]
        .concat();
        let theme = Theme {
            styles: Theme::parse_styles(&styles),
            ..Default::default()
        };
        // local: black text
        let local = rule(1, 1, &[prop(70, 5, &black)]);
        let obj = [ld(1, &local), ld(2, b"big"), vi(3, 80)].concat();
        let style = ObjectStyle::parse(&obj);
        assert_eq!(style.name, "big");
        let p = theme.resolve(&style, SELF, NORMAL);
        assert_eq!(p.color(prop::BACKGROUND_COLOR).unwrap().hex(), "#ffffff");
        assert_eq!(p.length(prop::FONT_SIZE).unwrap().value, 20.0);
        assert_eq!(p.color(prop::COLOR).unwrap().hex(), "#000000");
        let pressed = theme.resolve(&style, SELF, 3);
        assert_eq!(
            pressed.color(prop::BACKGROUND_COLOR).unwrap().hex(),
            "#000000"
        );
        // Unknown named style: theme default only.
        let plain = ObjectStyle {
            object_type: 80,
            ..Default::default()
        };
        assert_eq!(theme.resolve(&plain, SELF, NORMAL).len(), 2);
    }
}
