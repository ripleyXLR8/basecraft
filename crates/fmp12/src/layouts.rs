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
//! * `[4].[5].[L].[5].[obj]` key 1 = static text of a text object,
//!   key 16 = object name.
//!
//! Writing: [`encode_objects`] and [`patch_part`] re-encode edited layouts
//! from the original messages (see [`crate::save::save_layouts`]).

use std::collections::BTreeMap;

use crate::pb::{Field, MsgBuf};
use crate::{Chunk, Error, File, int_path, pb::Msg, text};

/// Part kinds (field 1). Header, leading sub-summary, body, footer and top
/// navigation are seen in the templates; the others follow FileMaker's
/// part order and are unverified.
pub mod part {
    pub const HEADER: u64 = 1;
    pub const LEADING_GRAND_SUMMARY: u64 = 2;
    pub const LEADING_SUB_SUMMARY: u64 = 3;
    pub const BODY: u64 = 4;
    pub const TRAILING_SUB_SUMMARY: u64 = 5;
    pub const TRAILING_GRAND_SUMMARY: u64 = 6;
    pub const FOOTER: u64 = 7;
    pub const TITLE_HEADER: u64 = 8;
    pub const TITLE_FOOTER: u64 = 9;
    pub const TOP_NAVIGATION: u64 = 12;
    pub const BOTTOM_NAVIGATION: u64 = 13;
}

/// Autosizing anchors: the top four bits of the object options (field 3,
/// `<Options>` in FileMaker's XML). Left and top are FileMaker's default.
pub mod anchor {
    pub const LEFT: u64 = 1 << 28;
    pub const TOP: u64 = 1 << 29;
    pub const RIGHT: u64 = 1 << 30;
    pub const BOTTOM: u64 = 1 << 31;
    pub const ALL: u64 = LEFT | TOP | RIGHT | BOTTOM;
}

/// Other object option bits, set one at a time in FileMaker's Layout mode
/// (Arrange › Lock, Inspector › Sliding & visibility) and read back.
pub const LOCKED: u64 = 0x2;
/// Sliding up based on all objects above.
pub const SLIDE_UP: u64 = 0x10;
pub const SLIDE_LEFT: u64 = 0x20;
pub const HIDE_WHEN_PRINTING: u64 = 0x200;

/// Object type (style field 3) of a part kind, as FileMaker gives a new part.
pub fn part_style_type(kind: u64) -> u32 {
    match kind {
        part::TITLE_HEADER => 31,
        part::HEADER => 32,
        part::LEADING_GRAND_SUMMARY => 33,
        part::BODY => 34,
        part::LEADING_SUB_SUMMARY => 101,
        part::TRAILING_SUB_SUMMARY => 201,
        part::TRAILING_GRAND_SUMMARY => 37,
        part::FOOTER => 38,
        part::TITLE_FOOTER => 39,
        part::TOP_NAVIGATION => 40,
        part::BOTTOM_NAVIGATION => 41,
        _ => 30,
    }
}

/// Position of a part kind in FileMaker's fixed part order (top to bottom).
pub fn part_rank(kind: u64) -> usize {
    [12, 8, 1, 2, 3, 4, 5, 6, 7, 9, 13]
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(5)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    /// Key of the part node `[4].[5].[L].[3].[P]`; 0 for a part not yet in
    /// the file.
    pub id: u32,
    /// See [`part`].
    pub kind: u64,
    pub top: f64,
    pub height: f64,
    pub style: String,
    /// Raw style message (field 10), see [`crate::theme::ObjectStyle`].
    pub style_raw: Vec<u8>,
    /// Break field of a sub-summary: (table occurrence id, field id).
    pub break_field: Option<(u32, u32)>,
}

/// Part field 6: `08 len 00 <TO ref> len <path-int field>`.
fn break_field(b: &[u8]) -> Option<(u32, u32)> {
    let to = crate::graph::to_ref(b.get(3..7)?)?;
    let n = *b.get(7)? as usize;
    Some((to, crate::path_int(b.get(8..8 + n)?)?))
}

/// Inverse of [`break_field`].
pub fn break_field_bytes(to: u32, field: u32) -> Vec<u8> {
    let [hi, lo] = (to.saturating_sub(128) as u16).to_be_bytes();
    let f = crate::save::path_elem(field);
    let mut rest = vec![0, 3, 0xD0, hi, lo, f.len() as u8];
    rest.extend(f);
    let mut out = vec![8, rest.len() as u8];
    out.extend(rest);
    out
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
    /// Object name (Inspector, Position, Name), empty if none.
    pub name: String,
    /// Containing object (tab panel, portal, button bar…), if any.
    pub parent: Option<u64>,
    /// Field 6 of containers (repeated: their direct children, ascending):
    /// the first child, the front tab panel of a tab control.
    pub active_child: Option<u64>,
    /// Field display: 0 edit box, 1 drop-down list, 2 pop-up menu, … (field 8.5).
    pub display: u64,
    /// Value list id attached to a field object (field 8.6).
    pub value_list: Option<u32>,
    /// Button label calculation (token stream, `crate::calc::decompile`).
    pub label: Option<Vec<u8>>,
    /// Portal: (table occurrence as in XML, rows shown, first row).
    pub portal: Option<(u64, u32, u32)>,
    /// Tab panel: offset and width of its tab (field 14, fields 2 and 3).
    pub tab: Option<(u32, u32)>,
    /// Raw button message (field 16), see [`crate::scripts::button_action`].
    pub button_raw: Option<Vec<u8>>,
    /// Button action: a single step (Perform Script for a script button).
    pub button: Option<crate::scripts::Step>,
}

impl Bounds {
    fn contains(&self, o: &Bounds) -> bool {
        o.top >= self.top - 0.5
            && o.left >= self.left - 0.5
            && o.bottom <= self.bottom + 0.5
            && o.right <= self.right + 0.5
    }
}

/// Sets `parent` on every object. Children's bounds are relative to their
/// container: use [`Layout::absolute`] for page coordinates. Ids are in pre-order and containers carry
/// their first child; a container's subtree is the run of following ids
/// that lie inside its bounds. Objects listed in `top` (object list field 1)
/// are top-level and close every open container.
fn link_parents(objects: &mut [LayoutObject], top: &std::collections::BTreeSet<u64>) {
    let mut order: Vec<usize> = (0..objects.len()).collect();
    order.sort_by_key(|&i| objects[i].id);
    let mut stack: Vec<usize> = Vec::new();
    for i in order {
        if top.contains(&objects[i].id) {
            stack.clear();
        }
        while let Some(&top) = stack.last() {
            let t = &objects[top];
            let started = objects[i].id > t.id;
            // Children's bounds are relative to their container.
            let relative = Bounds {
                top: 0.0,
                left: 0.0,
                bottom: t.bounds.bottom - t.bounds.top,
                right: t.bounds.right - t.bounds.left,
            };
            let b = &objects[i].bounds;
            // FileMaker's rules: a panel belongs to a tab control; no portal in a portal.
            let allowed = match objects[i].kind {
                12 => t.kind == 11,
                9 => t.kind != 9,
                _ => true,
            };
            if started && allowed && relative.contains(b) {
                break;
            }
            stack.pop();
        }
        objects[i].parent = stack.last().map(|&p| objects[p].id);
        if matches!(objects[i].kind, 8 | 9 | 11 | 12 | 18) {
            stack.push(i);
        }
    }
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

impl Layout {
    /// Bounds of an object in layout coordinates (adds its ancestors' origins).
    pub fn absolute(&self, o: &LayoutObject) -> Bounds {
        let mut b = o.bounds.clone();
        let mut p = o.parent;
        while let Some(pid) = p {
            let Some(parent) = self.objects.iter().find(|x| x.id == pid) else {
                break;
            };
            b.top += parent.bounds.top;
            b.bottom += parent.bounds.top;
            b.left += parent.bounds.left;
            b.right += parent.bounds.left;
            p = parent.parent;
        }
        b
    }
}

#[derive(Default)]
struct Raw {
    name: String,
    header: Vec<u8>,
    theme: u32,
    parts: BTreeMap<u32, Vec<u8>>,
    objects: BTreeMap<u32, Vec<u8>>,
    texts: BTreeMap<u32, String>,
    names: BTreeMap<u32, String>,
    labels: BTreeMap<u32, Vec<u8>>,
    /// Button step parameters: `[4].[5].[L].[5].[obj].[17]`.
    actions: BTreeMap<u32, crate::scripts::RawParams>,
}

fn style_name(m: &Msg) -> String {
    m.str(2).unwrap_or_default().to_string()
}

fn object(m: &Msg, r: &Raw) -> LayoutObject {
    let (texts, labels) = (&r.texts, &r.labels);
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
        name: u32::try_from(id)
            .ok()
            .and_then(|i| r.names.get(&i).cloned())
            .unwrap_or_default(),
        parent: None,
        label: u32::try_from(id).ok().and_then(|i| labels.get(&i).cloned()),
        display: m.msg(8).and_then(|f| f.uint(5)).unwrap_or(0),
        value_list: m.msg(8).and_then(|f| f.uint(6)).map(|v| v as u32),
        active_child: m.uint(6),
        portal: m.msg(15).map(|p| {
            (
                p.uint(2).unwrap_or(0),
                p.f64(6).unwrap_or(1.0) as u32,
                p.uint(5).unwrap_or(1) as u32,
            )
        }),
        tab: m
            .msg(14)
            .and_then(|t| Some((t.uint(2)? as u32, t.uint(3)? as u32))),
        button_raw: m.bytes(16).map(<[u8]>::to_vec),
        button: m.bytes(16).and_then(|b| {
            let params = u32::try_from(id)
                .ok()
                .and_then(|i| r.actions.get(&i).cloned())
                .unwrap_or_default();
            crate::scripts::button_action(b, params)
        }),
    }
}

pub fn layouts(file: &File) -> Result<Vec<Layout>, Error> {
    let mut raw: BTreeMap<u32, Raw> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        if let [4, 5, l, 5, obj, 17, rest @ ..] = p.as_slice() {
            let a = raw.entry(*l).or_default().actions.entry(*obj).or_default();
            crate::scripts::add_param(a, rest, c);
            return;
        }
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
            // Button label calculation: token stream (see `crate::calc`).
            [4, 5, l, 5, obj, 3] if *key == 5 => {
                raw.entry(*l)
                    .or_default()
                    .labels
                    .entry(*obj)
                    .or_default()
                    .extend_from_slice(data);
            }
            [4, 5, l, 5, obj] if *key == 1 => {
                raw.entry(*l).or_default().texts.insert(*obj, text(data));
            }
            [4, 5, l, 5, obj] if *key == 16 && !data.is_empty() => {
                raw.entry(*l).or_default().names.insert(*obj, text(data));
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
                .iter()
                .filter_map(|(id, b)| Some((*id, Msg::parse(b)?)))
                .map(|(id, m)| Part {
                    id,
                    kind: m.uint(1).unwrap_or(0),
                    top: m.f64(4).unwrap_or(0.0),
                    height: m.f64(5).unwrap_or(0.0),
                    style: m.msg(10).map(|s| style_name(&s)).unwrap_or_default(),
                    style_raw: m.bytes(10).unwrap_or_default().to_vec(),
                    break_field: m.bytes(6).and_then(break_field),
                })
                .collect();
            let blob: Vec<u8> = r.objects.values().flatten().copied().collect();
            let list = Msg::parse(&blob).unwrap_or_default();
            let mut objects: Vec<LayoutObject> = list.msgs(2).map(|o| object(&o, &r)).collect();
            let top = list
                .all(1)
                .filter_map(|v| match v {
                    crate::pb::Value::Varint(id) => Some(*id),
                    _ => None,
                })
                .collect();
            // Containers list their direct children (repeated field 6);
            // layouts without such lists fall back to the geometry.
            let mut kids: BTreeMap<u64, u64> = BTreeMap::new();
            for o in list.msgs(2) {
                let id = o.uint(2).unwrap_or(0);
                for v in o.all(6) {
                    if let crate::pb::Value::Varint(c) = v {
                        kids.insert(*c, id);
                    }
                }
            }
            if kids.is_empty() {
                link_parents(&mut objects, &top);
            } else {
                for o in objects.iter_mut() {
                    o.parent = kids.get(&o.id).copied();
                }
            }
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

/// Bounds message `{1 top, 2 left, 3 bottom, 4 right}` (doubles), patched
/// over `old` so that other fields stay.
fn bounds_msg(old: MsgBuf, b: &Bounds) -> MsgBuf {
    let mut m = old;
    for (f, v) in [(1, b.top), (2, b.left), (3, b.bottom), (4, b.right)] {
        m.set(f, Field::F64(v));
    }
    m
}

/// Button message field 3 (the step): `{1 id, 2 arg, 3 options, 4 the
/// 14-byte inline block}`, see [`crate::scripts::button_action`].
fn step_msg(old: MsgBuf, s: &crate::scripts::Step) -> MsgBuf {
    let mut m = old;
    let mut block = s.raw.get(6..20).unwrap_or_default().to_vec();
    block.resize(14, 0);
    m.set(1, Field::Varint(s.id as u64));
    m.set(2, Field::Varint(s.arg as u64));
    m.set(3, Field::Varint(s.options as u64));
    m.set(4, Field::Bytes(block));
    m
}

/// Field-object message (object field 8): `{2 {1 TO, 2 field, 3 repetition},
/// 5 display, 6 value list}`.
fn field_msg(old: MsgBuf, o: &LayoutObject) -> MsgBuf {
    let mut m = old;
    if let Some((to, field, rep)) = o.field {
        let mut r = m.msg(2);
        r.set(1, Field::Varint(to));
        r.set(2, Field::Varint(field));
        r.set(3, Field::Varint(rep));
        m.set_msg(2, &r);
    }
    m.set(5, Field::Varint(o.display));
    match o.value_list {
        Some(v) => m.set(6, Field::Varint(v as u64)),
        None => m.remove(6),
    }
    m
}

/// Replaces every occurrence of the repeated varint `field` with `values`,
/// where the first one was (or before the first higher field).
fn set_repeated(m: &mut MsgBuf, field: u32, values: &[u64]) {
    let at =
        m.0.iter()
            .position(|(f, _)| *f >= field)
            .unwrap_or(m.0.len());
    m.remove(field);
    for (i, v) in values.iter().enumerate() {
        m.0.insert(at + i, (field, Field::Varint(*v)));
    }
}

/// Tab control field 13.5 (the front panel, XML `default`): kept when it
/// is still one of `panels`, else the first.
fn set_front_panel(m: &mut MsgBuf, panels: &[u64]) {
    let mut t = m.msg(13);
    let cur = match t.get(5) {
        Some(Field::Varint(v)) => Some(*v),
        _ => None,
    };
    if cur.is_none_or(|c| !panels.contains(&c))
        && let Some(first) = panels.first()
    {
        t.set(5, Field::Varint(*first));
        m.set_msg(13, &t);
    }
}

/// Portal message (object field 15): `{1: 0, 2 TO, 3 options, 5 first row,
/// 6 rows (double), 7: 1}`, patched over `old`.
fn portal_msg(old: MsgBuf, (to, rows, first): (u64, u32, u32)) -> MsgBuf {
    let mut m = old;
    if m.0.is_empty() {
        // As in the templates' portals.
        m.set(1, Field::Varint(0));
        m.set(3, Field::Varint(273));
    }
    m.set(2, Field::Varint(to));
    m.set(5, Field::Varint(first as u64));
    m.set(6, Field::F64(rows as f64));
    if m.get(7).is_none() {
        m.set(7, Field::Varint(1));
    }
    m
}

/// Re-encodes one object message, patching only what differs between
/// `old` and `new`; an unchanged object keeps its exact bytes. `children`
/// is the new list of direct children (field 6) when it changed.
pub fn patch_object(
    raw: &[u8],
    old: &LayoutObject,
    new: &LayoutObject,
    children: Option<&[u64]>,
) -> Vec<u8> {
    let Some(mut m) = MsgBuf::parse(raw) else {
        return raw.to_vec();
    };
    let mut changed = false;
    if new.kind != old.kind {
        m.set(1, Field::Varint(new.kind));
        changed = true;
    }
    if new.options != old.options {
        m.set(3, Field::Varint(new.options));
        changed = true;
    }
    if let Some(c) = children {
        set_repeated(&mut m, 6, c);
        if new.kind == 11 {
            set_front_panel(&mut m, c);
        }
        changed = true;
    }
    if new.tab != old.tab
        && let Some((x, w)) = new.tab
    {
        let mut t = m.msg(14);
        t.set(2, Field::Varint(x as u64));
        t.set(3, Field::Varint(w as u64));
        m.set_msg(14, &t);
        changed = true;
    }
    if new.portal != old.portal
        && let Some(p) = new.portal
    {
        let pm = portal_msg(m.msg(15), p);
        m.set_msg(15, &pm);
        changed = true;
    }
    if new.bounds != old.bounds {
        let b = bounds_msg(m.msg(4), &new.bounds);
        m.set_msg(4, &b);
        changed = true;
    }
    if new.style_raw != old.style_raw {
        m.set(5, Field::Bytes(new.style_raw.clone()));
        changed = true;
    }
    if (new.field, new.display, new.value_list) != (old.field, old.display, old.value_list) {
        let f = field_msg(m.msg(8), new);
        m.set_msg(8, &f);
        changed = true;
    }
    if new.button != old.button {
        match &new.button {
            Some(s) => {
                let mut b = m.msg(16);
                let st = step_msg(b.msg(3), s);
                b.set_msg(3, &st);
                m.set_msg(16, &b);
            }
            None => m.remove(16),
        }
        changed = true;
    }
    if changed { m.encode() } else { raw.to_vec() }
}

/// Message of a new object, built on `proto` (an existing object of the
/// same kind and style type, whose unknown fields FileMaker expects) or
/// from scratch.
pub fn new_object(proto: Option<&[u8]>, o: &LayoutObject, children: &[u64]) -> Vec<u8> {
    let from_scratch = proto.is_none();
    let mut m = proto.and_then(MsgBuf::parse).unwrap_or_default();
    // Container links, portal setup and actions belong to the prototype.
    m.remove(6);
    let proto_portal = m.msg(15);
    m.remove(15);
    m.set(1, Field::Varint(o.kind));
    m.set(2, Field::Varint(o.id));
    if o.options != 0 {
        m.set(3, Field::Varint(o.options));
    } else if m.get(3).is_none() {
        m.set(3, Field::Varint(anchor::LEFT | anchor::TOP));
    }
    let b = bounds_msg(m.msg(4), &o.bounds);
    m.set_msg(4, &b);
    if o.style_raw.is_empty() {
        // Groups have no style.
        m.remove(5);
    } else {
        m.set(5, Field::Bytes(o.style_raw.clone()));
    }
    set_repeated(&mut m, 6, children);
    if o.field.is_some() {
        let mut f = m.msg(8);
        if f.0.is_empty() {
            // Options and display flags of the templates' edit boxes.
            f = MsgBuf(vec![
                (1, Field::Varint(1179872)),
                (2, Field::Bytes(Vec::new())),
                (3, Field::Varint(1)),
                (4, Field::Varint(0)),
                (5, Field::Varint(0)),
                (7, Field::Varint(1)),
                (8, Field::Varint(1)),
            ]);
        }
        let f = field_msg(f, o);
        m.set_msg(8, &f);
    } else {
        m.remove(8);
    }
    if let Some(p) = o.portal {
        let pm = portal_msg(proto_portal, p);
        m.set_msg(15, &pm);
    }
    if from_scratch {
        match o.kind {
            // Text options, as on the templates' text objects.
            2 => m.set_msg(7, &MsgBuf(vec![(1, Field::Varint(2))])),
            // Tab control: {1 options, 2, 3 tab width, 5 front panel}.
            11 => m.set_msg(
                13,
                &MsgBuf(vec![
                    (1, Field::Varint(264)),
                    (2, Field::Varint(0)),
                    (3, Field::Varint(45)),
                    (5, Field::Varint(children.first().copied().unwrap_or(0))),
                ]),
            ),
            _ => {}
        }
    }
    if o.kind == 11 {
        // The front panel must be one of its own: FileMaker does not draw a
        // tab control whose front panel is missing.
        set_front_panel(&mut m, children);
    }
    if o.kind == 12 {
        // Tab panel: {1: 1, 2 tab offset, 3 tab width, 4: 0}, filled in by
        // the caller through `tab`.
        if let Some((x, w)) = o.tab {
            m.set_msg(
                14,
                &MsgBuf(vec![
                    (1, Field::Varint(1)),
                    (2, Field::Varint(x as u64)),
                    (3, Field::Varint(w as u64)),
                    (4, Field::Varint(0)),
                ]),
            );
        }
    }
    match &o.button {
        Some(s) => {
            let mut b = m.msg(16);
            if b.0.is_empty() {
                // Seen on buttons: {1: 0, 2: 0, 3: step, 5: 16, 6: 0}.
                b.set(1, Field::Varint(0));
                b.set(2, Field::Varint(0));
            }
            let st = step_msg(b.msg(3), s);
            b.set_msg(3, &st);
            if b.get(5).is_none() {
                b.set(5, Field::Varint(16));
            }
            // A text button: no icon (field 4 = icon type, as XML
            // `IconData type`; 5 = its size).
            b.remove(4);
            b.set(6, Field::Varint(0));
            m.set_msg(16, &b);
        }
        None => m.remove(16),
    }
    m.encode()
}

/// The object list message of `new`, re-encoded from the original list
/// `blob` of `old`: field 1 = ids of the top-level objects, field 2 = the
/// objects, front to back. Unchanged objects keep their bytes, so
/// `encode_objects(b, l, l, …) == b`. `proto` gives a prototype message for
/// a new object.
pub fn encode_objects(
    blob: &[u8],
    old: &Layout,
    new: &Layout,
    proto: &dyn Fn(&LayoutObject) -> Option<Vec<u8>>,
) -> Vec<u8> {
    let top = MsgBuf::parse(blob).unwrap_or_default();
    let raws: BTreeMap<u64, &[u8]> = top
        .0
        .iter()
        .filter_map(|(f, v)| match (f, v) {
            (2, Field::Bytes(b)) => Some((Msg::parse(b)?.uint(2).unwrap_or(0), b.as_slice())),
            _ => None,
        })
        .collect();
    let olds: BTreeMap<u64, &LayoutObject> = old.objects.iter().map(|o| (o.id, o)).collect();
    let alive: std::collections::BTreeSet<u64> = new.objects.iter().map(|o| o.id).collect();
    let mut ids: Vec<u64> = top
        .0
        .iter()
        .filter_map(|(f, v)| match (f, v) {
            (1, Field::Varint(id)) if alive.contains(id) => Some(*id),
            _ => None,
        })
        .collect();
    for o in &new.objects {
        if o.parent.is_none() && !olds.contains_key(&o.id) && !ids.contains(&o.id) {
            ids.push(o.id);
        }
    }
    let ids: Vec<(u32, Field)> = ids.into_iter().map(|i| (1, Field::Varint(i))).collect();
    let children = |l: &Layout, id: u64| -> Vec<u64> {
        let mut v: Vec<u64> = l
            .objects
            .iter()
            .filter(|c| c.parent == Some(id))
            .map(|c| c.id)
            .collect();
        v.sort_unstable();
        v
    };
    let objects: Vec<(u32, Field)> = new
        .objects
        .iter()
        .map(|o| {
            let kids = children(new, o.id);
            let b = match (raws.get(&o.id), olds.get(&o.id)) {
                (Some(raw), Some(prev)) => {
                    let changed = (kids != children(old, o.id)).then_some(kids.as_slice());
                    patch_object(raw, prev, o, changed)
                }
                _ => new_object(proto(o).as_deref(), o, &kids),
            };
            (2, Field::Bytes(b))
        })
        .collect();
    // Same field order as the original: ids where the first id was, objects
    // where the first object was, other fields in place.
    let mut out = MsgBuf::default();
    let (mut ids_done, mut objs_done) = (false, false);
    for (f, v) in &top.0 {
        match f {
            1 | 2 => {
                if !ids_done {
                    out.0.extend(ids.iter().cloned());
                    ids_done = true;
                }
                if *f == 2 && !objs_done {
                    out.0.extend(objects.iter().cloned());
                    objs_done = true;
                }
            }
            _ => out.0.push((*f, v.clone())),
        }
    }
    if !ids_done {
        out.0.extend(ids);
    }
    if !objs_done {
        out.0.extend(objects);
    }
    out.encode()
}

/// Part message (`[4].[5].[L].[3].[P]` key 2) with `new`'s top, height and
/// style; an unchanged part keeps its exact bytes.
pub fn patch_part(raw: &[u8], old: &Part, new: &Part) -> Vec<u8> {
    let Some(mut m) = MsgBuf::parse(raw) else {
        return raw.to_vec();
    };
    if old == new {
        return raw.to_vec();
    }
    if old.kind != new.kind {
        m.set(1, Field::Varint(new.kind));
    }
    if old.top != new.top {
        m.set(4, Field::F64(new.top));
    }
    if old.height != new.height {
        m.set(5, Field::F64(new.height));
    }
    if old.style_raw != new.style_raw {
        m.set(10, Field::Bytes(new.style_raw.clone()));
    }
    if old.break_field != new.break_field {
        match new.break_field {
            Some((to, f)) => {
                m.set(6, Field::Bytes(break_field_bytes(to, f)));
                m.0.sort_by_key(|(f, _)| *f);
            }
            None => m.remove(6),
        }
    }
    m.encode()
}

/// Message of a new part, as FileMaker writes one: `{1 kind, 2: 0, 3: 0,
/// 4 top, 5 height, [6 break field], 10 style, 11 alternate-row style}`.
pub fn new_part(p: &Part) -> Vec<u8> {
    let ty = part_style_type(p.kind);
    let style = if p.style_raw.is_empty() {
        MsgBuf(vec![(3, Field::Varint(ty as u64))]).encode()
    } else {
        p.style_raw.clone()
    };
    let alt = if p.kind == part::BODY { 35 } else { ty };
    let mut m = MsgBuf(vec![
        (1, Field::Varint(p.kind)),
        (2, Field::Varint(0)),
        (3, Field::Varint(0)),
        (4, Field::F64(p.top)),
        (5, Field::F64(p.height)),
    ]);
    if let Some((to, f)) = p.break_field {
        m.0.push((6, Field::Bytes(break_field_bytes(to, f))));
    }
    m.0.push((10, Field::Bytes(style)));
    m.set_msg(11, &MsgBuf(vec![(3, Field::Varint(alt as u64))]));
    m.encode()
}
