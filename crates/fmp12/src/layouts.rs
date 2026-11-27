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
    /// Object name (Inspector, Position, Name), empty if none.
    pub name: String,
    /// Containing object (tab panel, portal, button bar…), if any.
    pub parent: Option<u64>,
    /// Field 6 of containers: one of their children (the front tab panel of a
    /// tab control; for other containers its meaning is not settled).
    pub active_child: Option<u64>,
    /// Field display: 0 edit box, 1 drop-down list, 2 pop-up menu, … (field 8.5).
    pub display: u64,
    /// Value list id attached to a field object (field 8.6).
    pub value_list: Option<u32>,
    /// Button label calculation (token stream, `crate::calc::decompile`).
    pub label: Option<Vec<u8>>,
    /// Portal: (table occurrence as in XML, rows shown, first row).
    pub portal: Option<(u64, u32, u32)>,
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
        if matches!(objects[i].kind, 9 | 11 | 12 | 18) {
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
            link_parents(&mut objects, &top);
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

/// Re-encodes one object message, patching only what differs between
/// `old` and `new`; an unchanged object keeps its exact bytes.
pub fn patch_object(raw: &[u8], old: &LayoutObject, new: &LayoutObject) -> Vec<u8> {
    let Some(mut m) = MsgBuf::parse(raw) else {
        return raw.to_vec();
    };
    let mut changed = false;
    if new.kind != old.kind {
        m.set(1, Field::Varint(new.kind));
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
pub fn new_object(proto: Option<&[u8]>, o: &LayoutObject) -> Vec<u8> {
    let mut m = proto.and_then(MsgBuf::parse).unwrap_or_default();
    // Container links, portal setup and actions belong to the prototype.
    m.remove(6);
    m.remove(15);
    m.set(1, Field::Varint(o.kind));
    m.set(2, Field::Varint(o.id));
    let b = bounds_msg(m.msg(4), &o.bounds);
    m.set_msg(4, &b);
    m.set(5, Field::Bytes(o.style_raw.clone()));
    if o.field.is_some() {
        let f = field_msg(m.msg(8), o);
        m.set_msg(8, &f);
    } else {
        m.remove(8);
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
                b.set(6, Field::Varint(0));
            }
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
    let objects: Vec<(u32, Field)> = new
        .objects
        .iter()
        .map(|o| {
            let b = match (raws.get(&o.id), olds.get(&o.id)) {
                (Some(raw), Some(old)) => patch_object(raw, old, o),
                _ => new_object(proto(o).as_deref(), o),
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
    if old.top != new.top {
        m.set(4, Field::F64(new.top));
    }
    if old.height != new.height {
        m.set(5, Field::F64(new.height));
    }
    if old.style_raw != new.style_raw {
        m.set(10, Field::Bytes(new.style_raw.clone()));
    }
    m.encode()
}
