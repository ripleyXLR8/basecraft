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
/// The object has conditional formatting (node `.[138]`); set by FileMaker
/// on every object with conditions, checked on the templates.
pub const HAS_CONDITIONS: u64 = 0x1;
/// The object has a tooltip (node `.[3]`), checked on the templates.
pub const HAS_TOOLTIP: u64 = 0x4000;
/// The object has a "Hide object when" calculation (node `.[141]`).
pub const HAS_HIDE: u64 = 0x4;
/// "Hide object when" applies in Find mode too.
pub const HIDE_IN_FIND: u64 = 0x100;
/// Sliding up: "resize enclosing part" too.
pub const RESIZE_PART: u64 = 0x40;
/// Sliding up based on the objects directly above only (with [`SLIDE_UP`]).
pub const SLIDE_DIRECTLY_ABOVE: u64 = 0x8000;

/// Object kinds (field 1, XML `kind`).
pub mod kind {
    pub const FIELD: u64 = 1;
    pub const TEXT: u64 = 2;
    pub const GRAPHIC: u64 = 3;
    pub const PORTAL: u64 = 9;
    pub const BUTTON: u64 = 10;
    pub const TAB_CONTROL: u64 = 11;
    pub const TAB_PANEL: u64 = 12;
    /// External object: web viewer (`WEBV`) or chart (`CHRT`), field 11.
    pub const EXTERNAL: u64 = 13;
    pub const POPOVER_BUTTON: u64 = 14;
    pub const POPOVER: u64 = 15;
    pub const SLIDE_CONTROL: u64 = 16;
    pub const SLIDE_PANEL: u64 = 17;
    pub const BUTTON_BAR: u64 = 18;
}

/// Type id of a web viewer in object field 11.1 (`WEBV`).
pub const WEB_VIEWER: u64 = 0x5745_4256;

/// Slide control option bits (object field 19.1).
pub mod slide {
    /// Swipe gestures off.
    pub const NO_SWIPE: u64 = 0x1;
    /// Navigation dots hidden.
    pub const NO_DOTS: u64 = 0x2;
}

/// Field object option bits (object field 8.1, the clipboard XML's
/// `FieldObj flags`; the templates' edit boxes have `0x1200E0`). Each was
/// set in FileMaker's Inspector › Data and read back.
pub mod field_flags {
    /// Drop-down list: "Include arrow icon to show and hide list".
    pub const ARROW: u64 = 0x400;
    /// Edit box: "Auto-complete using previously entered values"; drop-down
    /// list: "Auto-complete using value list".
    pub const AUTO_COMPLETE: u64 = 0x800;
    /// "Include field for Quick Find" unchecked (Inspector › Data › Behavior).
    pub const NO_QUICK_FIND: u64 = 0x8000;
}

/// A conditional format (`.[138].[128 + i]`): a test, and the style it
/// applies (`k13` = `{1 options, 2 style {1 local rules}, 3 operator}`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Condition {
    /// 0 formula; value comparisons, see [`op`].
    pub op: u64,
    /// The formula (token stream, node `.[5]`), for op 0.
    pub calc: Option<Vec<u8>>,
    /// The values compared with (keys 1 and 6, text).
    pub value1: String,
    pub value2: String,
    /// Raw `k13` message: options, style and operator.
    pub format: Vec<u8>,
}

/// Conditional format operators (k13 field 3): 0 formula, then the
/// "Value is" choices in the order of FileMaker's Conditional Formatting
/// dialog. FileMaker also writes a value test as a formula on `Self` in
/// `.[5]` (e.g. `Self = "hello"` for "is equal to hello"), which is what
/// it evaluates.
pub mod op {
    pub const FORMULA: u64 = 0;
    pub const BETWEEN: u64 = 1;
    pub const NOT_BETWEEN: u64 = 2;
    pub const EQUAL: u64 = 3;
    pub const NOT_EQUAL: u64 = 4;
    pub const GREATER: u64 = 5;
    pub const LESS: u64 = 6;
    pub const GREATER_EQUAL: u64 = 7;
    pub const LESS_EQUAL: u64 = 8;
    pub const CONTAINING: u64 = 9;
    pub const NOT_CONTAINING: u64 = 10;
    pub const BEGINNING: u64 = 11;
    pub const ENDING: u64 = 12;
    pub const EMPTY: u64 = 13;
    pub const BEFORE_TODAY: u64 = 14;
    pub const TODAY: u64 = 15;
    pub const AFTER_TODAY: u64 = 16;
    pub const MORE_DAYS_AFTER: u64 = 17;
    pub const MORE_DAYS_BEFORE: u64 = 18;
    pub const LESS_DAYS_AFTER: u64 = 19;
    pub const LESS_DAYS_BEFORE: u64 = 20;
}

/// The formula FileMaker stores for a value test (`.[5]` of the
/// condition), from the values as typed (numbers as they are, other
/// values quoted). `None` for a formula condition.
pub fn value_formula(op: u64, value1: &str, value2: &str) -> Option<String> {
    let lit = |v: &str| {
        let t = v.trim();
        if !t.is_empty() && t.parse::<f64>().is_ok() {
            t.to_string()
        } else {
            format!("\"{}\"", t.replace('"', "\"\""))
        }
    };
    let (a, b) = (lit(value1), lit(value2));
    let today = "Get ( CurrentDate )";
    Some(match op {
        op::BETWEEN => format!("Self \u{2265} {a} and Self \u{2264} {b}"),
        op::NOT_BETWEEN => format!("Self < {a} or Self > {b}"),
        op::EQUAL => format!("Self = {a}"),
        op::NOT_EQUAL => format!("Self \u{2260} {a}"),
        op::GREATER => format!("Self > {a}"),
        op::LESS => format!("Self < {a}"),
        op::GREATER_EQUAL => format!("Self \u{2265} {a}"),
        op::LESS_EQUAL => format!("Self \u{2264} {a}"),
        op::CONTAINING => format!("PatternCount ( Self ; {a} ) > 0"),
        op::NOT_CONTAINING => format!("PatternCount ( Self ; {a} ) = 0"),
        op::BEGINNING => format!("Left ( Self ; Length ( {a} ) ) = {a}"),
        op::ENDING => format!("Right ( Self ; Length ( {a} ) ) = {a}"),
        op::EMPTY => "IsEmpty ( Self )".into(),
        op::BEFORE_TODAY => format!("Self < {today}"),
        op::TODAY => format!("Self = {today}"),
        op::AFTER_TODAY => format!("Self > {today}"),
        op::MORE_DAYS_AFTER => format!("Self > {today} + {a}"),
        op::MORE_DAYS_BEFORE => format!("Self < {today} - {a}"),
        op::LESS_DAYS_AFTER => format!("Self < {today} + {a}"),
        op::LESS_DAYS_BEFORE => format!("Self > {today} - {a}"),
        _ => return None,
    })
}

impl Condition {
    /// The style it applies: an object style message (local rules in field 1).
    pub fn style(&self) -> Vec<u8> {
        Msg::parse(&self.format)
            .and_then(|m| m.bytes(2).map(<[u8]>::to_vec))
            .unwrap_or_default()
    }

    /// A condition applying `style` (an object style message, see
    /// [`crate::theme::ObjectStyle`]), as FileMaker writes it: options 3 when
    /// only the text colour is set (as seen), the operator in field 3.
    pub fn with_style(&self, style: &[u8]) -> Vec<u8> {
        let mut m = MsgBuf::parse(&self.format).unwrap_or_default();
        if m.get(1).is_none() {
            m.set(1, Field::Varint(3));
        }
        m.set(2, Field::Bytes(style.to_vec()));
        set_op(&mut m, self.op);
        m.encode()
    }
}

/// The operator: field 3 of a condition's `k13`, after the style.
fn set_op(m: &mut MsgBuf, op: u64) {
    m.remove(3);
    m.0.push((3, Field::Varint(op)));
}

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
    /// Field object options (field 8.1), see [`field_flags`].
    pub field_flags: u64,
    /// Button label calculation (token stream, `crate::calc::decompile`).
    pub label: Option<Vec<u8>>,
    /// Portal: (table occurrence as in XML, rows shown, first row).
    pub portal: Option<(u64, u32, u32)>,
    /// Tab panel: offset and width of its tab (field 14, fields 2 and 3).
    pub tab: Option<(u32, u32)>,
    /// Raw button message (field 16), see [`crate::scripts::button_action`].
    pub button_raw: Option<Vec<u8>>,
    /// Graphic object (kind 3): its picture in the file's picture store
    /// `[31].[5].[id]` ([`crate::containers::layout_pictures`]), field 10.1.
    pub picture: Option<u32>,
    /// Button action: a single step (Perform Script for a script button).
    pub button: Option<crate::scripts::Step>,
    /// Tooltip calculation (node `.[3]`).
    pub tooltip: Option<Vec<u8>>,
    /// Placeholder text calculation of a field (node `.[6]`).
    pub placeholder: Option<Vec<u8>>,
    /// "Hide object when" calculation (node `.[141]`).
    pub hide: Option<Vec<u8>>,
    /// Data formatting (node key 13), see [`crate::formats`].
    pub format: Option<crate::formats::Format>,
    /// Conditional formatting, in order (node `.[138]`).
    pub conditions: Vec<Condition>,
    /// Web viewer address calculation (node `.[128].[5]`).
    pub url: Option<Vec<u8>>,
    /// External object (field 11): type (`WEBV`, `CHRT`) and options.
    pub external: Option<(u64, u64)>,
    /// Popover button: its popover (field 17.3); popover: its button (18.4).
    pub popover: Option<u64>,
    /// Options of popovers (18.1), slide controls (19.1, see [`slide`]),
    /// slide panels (20.1) and button bars (21.1).
    pub flags: u64,
    /// Button bar: the active segment (field 21.2).
    pub selected: Option<u64>,
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
    /// Tab order (`[L].[21]`): object ids, first to last.
    pub tab_order: Vec<u64>,
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
    /// Calculations under an object's node: sub-path → token stream (k5).
    calcs: BTreeMap<u32, BTreeMap<Vec<u32>, Vec<u8>>>,
    /// Keys of an object's node (7 tab order, 13 formats).
    keys: BTreeMap<u32, BTreeMap<u32, Vec<u8>>>,
    /// Keys of conditional formats: (object, 128 + i) → key → value.
    conds: BTreeMap<(u32, u32), BTreeMap<u32, Vec<u8>>>,
    /// Tab order `[L].[21]`: key → object id.
    tabs: BTreeMap<u32, u64>,
    /// Button step parameters: `[4].[5].[L].[5].[obj].[17]`.
    actions: BTreeMap<u32, crate::scripts::RawParams>,
}

fn style_name(m: &Msg) -> String {
    m.str(2).unwrap_or_default().to_string()
}

fn object(m: &Msg, r: &Raw) -> LayoutObject {
    let texts = &r.texts;
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
        label: None,
        display: m.msg(8).and_then(|f| f.uint(5)).unwrap_or(0),
        value_list: m.msg(8).and_then(|f| f.uint(6)).map(|v| v as u32),
        field_flags: m.msg(8).and_then(|f| f.uint(1)).unwrap_or(0),
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
        button: m
            .bytes(16)
            .and_then(|b| {
                let params = u32::try_from(id)
                    .ok()
                    .and_then(|i| r.actions.get(&i).cloned())
                    .unwrap_or_default();
                crate::scripts::button_action(b, params)
            })
            .or_else(|| {
                let params = u32::try_from(id)
                    .ok()
                    .and_then(|i| r.actions.get(&i).cloned())
                    .unwrap_or_default();
                crate::scripts::legacy_button_action(m.bytes(12)?, params)
            }),
        picture: m
            .msg(10)
            .filter(|_| m.uint(1) == Some(kind::GRAPHIC))
            .and_then(|g| g.uint(1))
            .map(|v| v as u32),
        external: m
            .msg(11)
            .map(|x| (x.uint(1).unwrap_or(0), x.uint(3).unwrap_or(0))),
        popover: m
            .msg(17)
            .and_then(|x| x.uint(3))
            .or_else(|| m.msg(18).and_then(|x| x.uint(4))),
        flags: [18, 19, 20, 21]
            .iter()
            .find_map(|f| m.msg(*f).and_then(|x| x.uint(1)))
            .unwrap_or(0),
        selected: m.msg(21).and_then(|x| x.uint(2)),
        ..Default::default()
    }
    .with_node(r)
}

impl LayoutObject {
    /// Fills what the object's node `[4].[5].[L].[5].[id]` holds.
    fn with_node(mut self, r: &Raw) -> Self {
        let Ok(id) = u32::try_from(self.id) else {
            return self;
        };
        let calc = |sub: &[u32]| r.calcs.get(&id).and_then(|c| c.get(sub)).cloned();
        // Tab panels keep their label in `text` (node key 1 and `.[2]`).
        if self.kind != kind::TAB_PANEL {
            self.label = calc(&[2]);
        }
        self.tooltip = calc(&[3]);
        self.placeholder = calc(&[6]);
        self.hide = calc(&[141]);
        if self.external.is_some_and(|e| e.0 == WEB_VIEWER) {
            self.url = calc(&[128, 5]);
        }
        let keys = r.keys.get(&id);
        self.format = keys
            .and_then(|k| k.get(&13))
            .and_then(|b| crate::formats::Format::parse(b));
        self.conditions = r
            .conds
            .range((id, 0)..=(id, u32::MAX))
            .map(|((_, i), k)| {
                let get = |key: u32| k.get(&key).cloned().unwrap_or_default();
                let format = get(13);
                let op = Msg::parse(&format)
                    .and_then(|m| m.all(3).last().copied())
                    .and_then(|v| match v {
                        crate::pb::Value::Varint(x) => Some(x),
                        _ => None,
                    })
                    .unwrap_or(0);
                Condition {
                    op,
                    calc: calc(&[138, *i, 5]),
                    value1: crate::text(&get(1)),
                    value2: crate::text(&get(6)),
                    format,
                }
            })
            .collect();
        self
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
            // Calculations under an object's node: token stream (`crate::calc`).
            [4, 5, l, 5, obj, sub @ ..] if *key == 5 && !sub.is_empty() => {
                raw.entry(*l)
                    .or_default()
                    .calcs
                    .entry(*obj)
                    .or_default()
                    .entry(sub.to_vec())
                    .or_default()
                    .extend_from_slice(data);
            }
            [4, 5, l, 5, obj, 138, i] => {
                raw.entry(*l)
                    .or_default()
                    .conds
                    .entry((*obj, *i))
                    .or_default()
                    .insert(*key, data.to_vec());
            }
            [4, 5, l, 21] => {
                if let Some(id) = crate::save::index_ref(data) {
                    raw.entry(*l).or_default().tabs.insert(*key, id as u64);
                }
            }
            [4, 5, l, 5, obj] if matches!(key, 7 | 13) => {
                raw.entry(*l)
                    .or_default()
                    .keys
                    .entry(*obj)
                    .or_default()
                    .insert(*key, data.to_vec());
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
                tab_order: r.tabs.values().copied().collect(),
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
    if o.field_flags != 0 {
        m.set(1, Field::Varint(o.field_flags));
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
    if effective_options(new) != effective_options(old) {
        m.set(3, Field::Varint(effective_options(new)));
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
    if (new.field, new.display, new.value_list, new.field_flags)
        != (old.field, old.display, old.value_list, old.field_flags)
    {
        let f = field_msg(m.msg(8), new);
        m.set_msg(8, &f);
        changed = true;
    }
    if (new.external, new.popover, new.flags, new.selected)
        != (old.external, old.popover, old.flags, old.selected)
    {
        kind_fields(&mut m, new);
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

/// Options with the bits FileMaker keeps in step with the object's node:
/// [`HAS_CONDITIONS`], [`HAS_TOOLTIP`] and [`HAS_HIDE`].
pub fn effective_options(o: &LayoutObject) -> u64 {
    let mut v = o.options & !(HAS_CONDITIONS | HAS_TOOLTIP | HAS_HIDE);
    if o.hide.is_some() {
        v |= HAS_HIDE;
    }
    if !o.conditions.is_empty() {
        v |= HAS_CONDITIONS;
    }
    if o.tooltip.is_some() {
        v |= HAS_TOOLTIP;
    }
    v
}

/// The fields of popover buttons (17), popovers (18), slide controls (19),
/// slide panels (20), button bars (21) and external objects (11), as
/// FileMaker writes them for new objects (Insert menu, FileMaker Pro 26).
fn kind_fields(m: &mut MsgBuf, o: &LayoutObject) {
    let set1 = |m: &mut MsgBuf, f: u32, extra: &[(u32, u64)]| {
        let mut x = m.msg(f);
        x.set(1, Field::Varint(o.flags));
        for (k, v) in extra {
            x.set(*k, Field::Varint(*v));
        }
        m.set_msg(f, &x);
    };
    match o.kind {
        kind::POPOVER_BUTTON => {
            let mut x = m.msg(17);
            if x.0.is_empty() {
                x = MsgBuf(vec![
                    (1, Field::Varint(0)),
                    (2, Field::Varint(2)),
                    (3, Field::Varint(0)),
                    (5, Field::Varint(12)),
                    (6, Field::Varint(0)),
                ]);
            }
            x.set(3, Field::Varint(o.popover.unwrap_or(0)));
            m.set_msg(17, &x);
        }
        kind::POPOVER => {
            let x = m.msg(18);
            let pos = match x.get(3) {
                Some(Field::Varint(v)) => *v,
                _ => 0,
            };
            set1(m, 18, &[(3, pos), (4, o.popover.unwrap_or(0))]);
        }
        kind::SLIDE_CONTROL => set1(m, 19, &[]),
        kind::SLIDE_PANEL => set1(m, 20, &[]),
        kind::BUTTON_BAR => {
            set1(m, 21, &[]);
            let mut x = m.msg(21);
            match o.selected {
                Some(s) => x.set(2, Field::Varint(s)),
                None => x.remove(2),
            }
            m.set_msg(21, &x);
        }
        kind::EXTERNAL => {
            if let Some((ty, flags)) = o.external {
                let mut x = m.msg(11);
                x.set(1, Field::Varint(ty));
                if x.get(2).is_none() {
                    x.set(2, Field::Varint(0));
                }
                x.set(3, Field::Varint(flags));
                m.set_msg(11, &x);
            }
        }
        _ => {}
    }
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
        m.set(3, Field::Varint(effective_options(o)));
    } else if m.get(3).is_none() {
        m.set(
            3,
            Field::Varint(effective_options(&LayoutObject {
                options: anchor::LEFT | anchor::TOP,
                ..o.clone()
            })),
        );
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
    kind_fields(&mut m, o);
    match &o.button {
        None if o.kind == kind::BUTTON && m.get(16).is_none() => {
            // A button without an action (a new button bar segment), as
            // FileMaker writes one: an empty step and its inline block.
            let step = MsgBuf(vec![
                (1, Field::Varint(0)),
                (2, Field::Varint(0)),
                (3, Field::Varint(0)),
                (4, Field::Bytes(vec![0; 14])),
            ]);
            let mut b = MsgBuf(vec![(1, Field::Varint(0)), (2, Field::Varint(2))]);
            b.set_msg(3, &step);
            b.set(5, Field::Varint(12));
            b.set(6, Field::Varint(0));
            m.set_msg(16, &b);
        }
        None if o.kind == kind::BUTTON => {}
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
