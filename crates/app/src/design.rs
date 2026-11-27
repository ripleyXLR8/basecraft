//! Layout mode (M6): the layout canvas with rulers, grid and part labels,
//! the tool palette, selection, move and resize, part resizing, undo/redo,
//! the Inspector, and the dialogs that place fields and buttons.
//!
//! The designer edits a copy of the layout. FileMaker asks whether to save
//! when the user leaves the layout; "Save" copies the edit into the open
//! document, and File › Save writes it to the `.fmp12` through
//! `fmp12::save::save_layouts`.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use fmp12::layouts::{Bounds, Layout, LayoutObject, Part};
use fmp12::theme::{self, Length, NORMAL, ObjectStyle, Rgba, SELF, TEXT, Unit, prop};

use crate::Doc;
use crate::i18n::tr;
use crate::look::{self, Look};

const RULER: f32 = 18.0;
/// Left margin holding the part labels.
const GUTTER: f32 = 96.0;
const GRID: f32 = 12.0;
const HANDLE: f32 = 6.0;

/// Local style of a new object: `{1: local rules (613 = 0, as FileMaker
/// writes in every local style)}`, then field 3 = object type.
const LOCAL_613: [u8; 17] = [
    0x0a, 0x0f, 0x0a, 0x0d, 0x08, 0x01, 0x10, 0x01, 0x1a, 0x07, 0x0a, 0x05, 0x08, 0xe5, 0x04, 0x18,
    0x00,
];

fn new_style(object_type: u8) -> Vec<u8> {
    let mut s = LOCAL_613.to_vec();
    s.extend([0x18, object_type]);
    s
}

/// Object types of the field control styles (display 0–3).
const DISPLAY_TYPES: [u32; 4] = [61, 62, 63, 64];
const DISPLAY_NAMES: [&str; 4] = ["Edit box", "Drop-down list", "Pop-up menu", "Checkbox set"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Text,
    Field,
    Rect,
    Line,
    Button,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Position,
    Appearance,
    Data,
}

/// What to do once the "save changes?" question is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    Exit,
    Switch(usize),
    NewLayout,
}

enum Drag {
    Move {
        start: Pos2,
        orig: Layout,
    },
    Resize {
        id: u64,
        handle: usize,
        start: Pos2,
        orig: Layout,
    },
    Part {
        index: usize,
        start: Pos2,
        orig: Layout,
    },
    Marquee {
        start: Pos2,
        now: Pos2,
        add: bool,
    },
    Create {
        start: Pos2,
        now: Pos2,
    },
}

enum Dialog {
    /// Specify Field: rectangle drawn, table occurrence, field, create a label.
    Field {
        rect: Bounds,
        to: u32,
        field: Option<u32>,
        label: bool,
    },
    /// Button Setup: rectangle (or the button edited), label, action.
    Button {
        rect: Bounds,
        edit: Option<u64>,
        label: String,
        script: bool,
        target: u32,
    },
}

pub struct Designer {
    /// Index of the layout in `Doc::layouts`.
    pub index: usize,
    pub layout: Layout,
    saved: Layout,
    sel: BTreeSet<u64>,
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    tool: Tool,
    drag: Option<Drag>,
    tab: Tab,
    text_edit: Option<(u64, String)>,
    dialog: Option<Dialog>,
    /// Layout Setup dialog: name and table occurrence being edited.
    setup: Option<(String, u64)>,
    /// Pending "save changes?" question.
    pub ask: Option<Then>,
    /// Inspector property last edited: consecutive edits make one undo step.
    touched: Option<&'static str>,
    /// Display names of the named styles seen in the file (CSS name → name).
    displays: BTreeMap<String, String>,
    next_id: u64,
}

fn rect_of(b: &Bounds) -> Rect {
    Rect::from_min_max(
        egui::pos2(b.left as f32, b.top as f32),
        egui::pos2(b.right as f32, b.bottom as f32),
    )
}

fn part_name(kind: u64) -> &'static str {
    // ponytail: kinds 1, 3, 4, 7 and 12 are seen in the templates; the
    // others follow FileMaker's part order and are unverified.
    tr(match kind {
        1 => "Header",
        2 => "Leading Grand Summary",
        3 | 5 => "Sub-summary",
        4 => "Body",
        6 => "Trailing Grand Summary",
        7 => "Footer",
        8 => "Title Header",
        9 => "Title Footer",
        12 => "Top Navigation",
        13 => "Bottom Navigation",
        _ => "Part",
    })
}

/// Ids of `o`'s containers, innermost first.
fn ancestors(l: &Layout, o: &LayoutObject) -> Vec<u64> {
    let mut v = Vec::new();
    let mut p = o.parent;
    while let Some(id) = p {
        if v.contains(&id) {
            break;
        }
        v.push(id);
        p = l.objects.iter().find(|x| x.id == id).and_then(|x| x.parent);
    }
    v
}

/// Front panel of a tab control: field 6, else its first panel.
fn front_panel(l: &Layout, tc: &LayoutObject) -> Option<u64> {
    tc.active_child.or_else(|| {
        l.objects
            .iter()
            .filter(|x| x.parent == Some(tc.id) && x.kind == 12)
            .map(|x| x.id)
            .min()
    })
}

/// False for objects on a tab panel that is not in front.
fn visible(l: &Layout, o: &LayoutObject) -> bool {
    ancestors(l, o).iter().all(|a| {
        let Some(p) = l.objects.iter().find(|x| x.id == *a) else {
            return true;
        };
        if p.kind != 12 {
            return true;
        }
        let tc = p.parent.and_then(|t| l.objects.iter().find(|x| x.id == t));
        tc.is_none_or(|tc| front_panel(l, tc) == Some(p.id))
    })
}

fn handles(r: Rect) -> [Pos2; 8] {
    let (c, m) = (r.center(), r.min);
    let x = r.max;
    [
        m,
        egui::pos2(c.x, m.y),
        egui::pos2(x.x, m.y),
        egui::pos2(x.x, c.y),
        x,
        egui::pos2(c.x, x.y),
        egui::pos2(m.x, x.y),
        egui::pos2(m.x, c.y),
    ]
}

fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let f = |x: u8| x as f32 / 255.0;
    Rgba {
        r: f(r),
        g: f(g),
        b: f(b),
        a: f(a),
    }
}

fn pt(v: f64) -> theme::Value {
    theme::Value::Length(Length {
        value: v,
        unit: Unit::Pt,
    })
}

impl Designer {
    pub fn new(doc: &Doc, index: usize) -> Self {
        let layout = doc.layouts[index].clone();
        let mut displays = BTreeMap::new();
        for l in &doc.db.layouts {
            for o in &l.objects {
                if let Some(m) = fmp12::pb::Msg::parse(&o.style_raw)
                    && let (Some(n), Some(d)) = (m.str(2), m.str(4))
                {
                    displays.insert(n.to_string(), d.to_string());
                }
            }
        }
        let next_id = layout.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        Designer {
            index,
            saved: layout.clone(),
            layout,
            sel: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            tool: Tool::Select,
            drag: None,
            tab: Tab::Position,
            text_edit: None,
            dialog: None,
            setup: None,
            ask: None,
            touched: None,
            displays,
            next_id,
        }
    }

    /// Whether the layout differs from its saved state.
    pub fn changed(&self) -> bool {
        self.finish_text_preview() != self.saved
    }

    fn finish_text_preview(&self) -> Layout {
        let mut l = self.layout.clone();
        if let Some((id, t)) = &self.text_edit
            && let Some(o) = l.objects.iter_mut().find(|o| o.id == *id)
        {
            o.text = Some(t.clone());
        }
        l
    }

    /// Copies the edited layout into the document (FileMaker's "Save").
    pub fn apply(&mut self, doc: &mut Doc) {
        self.end_text_edit();
        doc.layouts[self.index] = self.layout.clone();
        match doc.db.layouts.iter_mut().find(|l| l.id == self.layout.id) {
            Some(l) => *l = self.layout.clone(),
            None => doc.db.layouts.push(self.layout.clone()),
        }
        self.saved = self.layout.clone();
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.layout.clone());
        self.redo.clear();
        self.touched = None;
    }

    /// Checkpoint for an Inspector edit of `what` (coalesced).
    fn touch(&mut self, what: &'static str) {
        if self.touched != Some(what) {
            self.checkpoint();
            self.touched = Some(what);
        }
    }

    pub fn undo(&mut self) {
        self.end_text_edit();
        if let Some(l) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.layout, l));
            self.prune_selection();
        }
        self.touched = None;
    }

    pub fn redo(&mut self) {
        if let Some(l) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.layout, l));
            self.prune_selection();
        }
        self.touched = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn prune_selection(&mut self) {
        let ids: BTreeSet<u64> = self.layout.objects.iter().map(|o| o.id).collect();
        self.sel.retain(|i| ids.contains(i));
    }

    pub fn select_all(&mut self) {
        self.sel = self
            .layout
            .objects
            .iter()
            .filter(|o| o.parent.is_none())
            .map(|o| o.id)
            .collect();
    }

    /// Deletes the selected objects and everything inside them.
    pub fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let l = &self.layout;
        let gone: BTreeSet<u64> = l
            .objects
            .iter()
            .filter(|o| {
                self.sel.contains(&o.id) || ancestors(l, o).iter().any(|a| self.sel.contains(a))
            })
            .map(|o| o.id)
            .collect();
        self.layout.objects.retain(|o| !gone.contains(&o.id));
        self.sel.clear();
    }

    /// Selected objects not inside another selected object: moving a
    /// container moves its contents (their bounds are relative).
    fn movable(&self) -> BTreeSet<u64> {
        let l = &self.layout;
        l.objects
            .iter()
            .filter(|o| self.sel.contains(&o.id))
            .filter(|o| !ancestors(l, o).iter().any(|a| self.sel.contains(a)))
            .map(|o| o.id)
            .collect()
    }

    fn nudge(&mut self, dx: f64, dy: f64) {
        let ids = self.movable();
        if ids.is_empty() {
            return;
        }
        if self.touched != Some("nudge") {
            self.checkpoint();
            self.touched = Some("nudge");
        }
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| ids.contains(&o.id))
        {
            o.bounds.top += dy;
            o.bounds.bottom += dy;
            o.bounds.left += dx;
            o.bounds.right += dx;
        }
    }

    /// Keyboard: arrows move (Shift = 10 pt), Delete, Escape, undo/redo.
    pub fn keys(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || self.setup.is_some() || self.ask.is_some() {
            return;
        }
        if self.text_edit.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.end_text_edit();
            }
            return;
        }
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let (redo, undo, all) = ctx.input_mut(|i| {
            let r = i.consume_key(cmd | Modifiers::SHIFT, Key::Z) || i.consume_key(cmd, Key::Y);
            (r, i.consume_key(cmd, Key::Z), i.consume_key(cmd, Key::A))
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if all {
            self.select_all();
        }
        let (step, del, esc, arrows) = ctx.input_mut(|i| {
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
            let del = i.consume_key(Modifiers::NONE, Key::Delete)
                || i.consume_key(Modifiers::NONE, Key::Backspace);
            let esc = i.consume_key(Modifiers::NONE, Key::Escape);
            let mut a = (0.0, 0.0);
            for (k, d) in [
                (Key::ArrowLeft, (-1.0, 0.0)),
                (Key::ArrowRight, (1.0, 0.0)),
                (Key::ArrowUp, (0.0, -1.0)),
                (Key::ArrowDown, (0.0, 1.0)),
            ] {
                while i.consume_key(Modifiers::NONE, k) {
                    a.0 += d.0;
                    a.1 += d.1;
                }
            }
            (step, del, esc, a)
        });
        if arrows != (0.0, 0.0) {
            self.nudge(arrows.0 * step, arrows.1 * step);
        }
        if del {
            self.delete_selection();
        }
        if esc {
            self.sel.clear();
            self.tool = Tool::Select;
        }
    }

    fn end_text_edit(&mut self) {
        if let Some((id, t)) = self.text_edit.take() {
            let cur = self
                .layout
                .objects
                .iter()
                .find(|o| o.id == id)
                .and_then(|o| o.text.clone());
            if cur.as_deref() != Some(t.as_str()) {
                self.checkpoint();
                if let Some(o) = self.layout.objects.iter_mut().find(|o| o.id == id) {
                    o.text = Some(t);
                }
            }
        }
    }

    /// Tool palette shown in the status toolbar.
    pub fn tools(&mut self, ui: &mut egui::Ui) {
        for (t, icon, tip) in [
            (Tool::Select, "↖", "Selection tool"),
            (Tool::Text, "T", "Text tool"),
            (Tool::Field, "[F]", "Field tool"),
            (Tool::Rect, "▭", "Rectangle tool"),
            (Tool::Line, "╱", "Line tool"),
            (Tool::Button, "(B)", "Button tool"),
        ] {
            if ui
                .selectable_label(self.tool == t, icon)
                .on_hover_text(tr(tip))
                .clicked()
            {
                self.end_text_edit();
                self.tool = t;
            }
        }
        ui.separator();
        if ui
            .add_enabled(self.can_undo(), egui::Button::new("↶"))
            .on_hover_text(tr("Undo"))
            .clicked()
        {
            self.undo();
        }
        if ui
            .add_enabled(self.can_redo(), egui::Button::new("↷"))
            .on_hover_text(tr("Redo"))
            .clicked()
        {
            self.redo();
        }
    }

    pub fn open_setup(&mut self) {
        self.setup = Some((self.layout.name.clone(), self.layout.table_occurrence));
    }

    fn alloc_id(&mut self) -> u64 {
        let max = self.layout.objects.iter().map(|o| o.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
        self.next_id += 1;
        self.next_id - 1
    }

    /// Adds a new object on top of the others and selects it.
    fn add(&mut self, mut o: LayoutObject) -> u64 {
        o.id = self.alloc_id();
        let id = o.id;
        self.layout.objects.insert(0, o);
        self.sel = BTreeSet::from([id]);
        id
    }

    fn object(&self, id: u64) -> Option<&LayoutObject> {
        self.layout.objects.iter().find(|o| o.id == id)
    }

    /// Topmost visible object under `p` (layout coordinates), preferring
    /// the innermost one (a field in a portal rather than the portal).
    fn hit(&self, p: Pos2) -> Option<u64> {
        let l = &self.layout;
        l.objects
            .iter()
            .filter(|o| o.kind != 12 && visible(l, o))
            .filter(|o| rect_of(&l.absolute(o)).expand(3.0).contains(p))
            .max_by_key(|o| {
                let depth = ancestors(l, o).len();
                // Earlier in the list = more in front; max_by_key keeps the last max.
                let pos = l.objects.iter().position(|x| x.id == o.id).unwrap_or(0);
                (depth, usize::MAX - pos)
            })
            .map(|o| o.id)
    }

    fn handle_at(&self, p: Pos2) -> Option<(u64, usize)> {
        for id in &self.sel {
            let Some(o) = self.object(*id) else { continue };
            let r = rect_of(&self.layout.absolute(o));
            if let Some(h) = handles(r).iter().position(|h| h.distance(p) <= HANDLE) {
                return Some((*id, h));
            }
        }
        None
    }

    /// Part whose bottom boundary is at `y`.
    fn boundary_at(&self, y: f32) -> Option<usize> {
        self.layout
            .parts
            .iter()
            .position(|p| ((p.top + p.height) as f32 - y).abs() <= 3.0)
    }

    fn press(&mut self, p: Pos2, shift: bool, in_gutter: bool) {
        self.end_text_edit();
        self.touched = None;
        if self.tool != Tool::Select {
            self.drag = Some(Drag::Create { start: p, now: p });
            return;
        }
        if let Some((id, handle)) = self.handle_at(p) {
            self.drag = Some(Drag::Resize {
                id,
                handle,
                start: p,
                orig: self.layout.clone(),
            });
            return;
        }
        if let Some(index) = self
            .boundary_at(p.y)
            .filter(|_| in_gutter || self.hit(p).is_none())
        {
            self.drag = Some(Drag::Part {
                index,
                start: p,
                orig: self.layout.clone(),
            });
            return;
        }
        match self.hit(p) {
            Some(id) => {
                if shift {
                    if !self.sel.remove(&id) {
                        self.sel.insert(id);
                    }
                } else if !self.sel.contains(&id) {
                    self.sel = BTreeSet::from([id]);
                }
                self.drag = Some(Drag::Move {
                    start: p,
                    orig: self.layout.clone(),
                });
            }
            None => {
                if !shift {
                    self.sel.clear();
                }
                self.drag = Some(Drag::Marquee {
                    start: p,
                    now: p,
                    add: shift,
                });
            }
        }
    }

    fn drag_to(&mut self, p: Pos2) {
        let movable = self.movable();
        match &mut self.drag {
            Some(Drag::Move { start, orig }) => {
                let d = (p - *start).round();
                let mut l = orig.clone();
                for o in l.objects.iter_mut().filter(|o| movable.contains(&o.id)) {
                    o.bounds.top += d.y as f64;
                    o.bounds.bottom += d.y as f64;
                    o.bounds.left += d.x as f64;
                    o.bounds.right += d.x as f64;
                }
                self.layout = l;
            }
            Some(Drag::Resize {
                id,
                handle,
                start,
                orig,
            }) => {
                let d = (p - *start).round();
                let (dx, dy) = (d.x as f64, d.y as f64);
                let mut l = orig.clone();
                if let Some(o) = l.objects.iter_mut().find(|o| o.id == *id) {
                    let b = &mut o.bounds;
                    if matches!(handle, 0 | 6 | 7) {
                        b.left = (b.left + dx).min(b.right - 1.0);
                    }
                    if matches!(handle, 2..=4) {
                        b.right = (b.right + dx).max(b.left + 1.0);
                    }
                    if matches!(handle, 0..=2) {
                        b.top = (b.top + dy).min(b.bottom);
                    }
                    if matches!(handle, 4..=6) {
                        b.bottom = (b.bottom + dy).max(b.top);
                    }
                }
                self.layout = l;
            }
            Some(Drag::Part { index, start, orig }) => {
                let p0 = &orig.parts[*index];
                let old_bottom = p0.top + p0.height;
                let dy = ((p.y - start.y).round() as f64).max(1.0 - p0.height);
                let mut l = orig.clone();
                l.parts[*index].height += dy;
                for (j, part) in l.parts.iter_mut().enumerate() {
                    if j != *index && part.top >= old_bottom - 0.5 {
                        part.top += dy;
                    }
                }
                // Objects in the parts below move with them.
                for o in l.objects.iter_mut().filter(|o| o.parent.is_none()) {
                    if o.bounds.top >= old_bottom - 0.5 {
                        o.bounds.top += dy;
                        o.bounds.bottom += dy;
                    }
                }
                self.layout = l;
            }
            Some(Drag::Marquee { now, .. }) | Some(Drag::Create { now, .. }) => *now = p,
            None => {}
        }
    }

    fn release(&mut self) {
        match self.drag.take() {
            Some(Drag::Move { orig, .. } | Drag::Resize { orig, .. } | Drag::Part { orig, .. }) => {
                if orig != self.layout {
                    self.undo.push(orig);
                    self.redo.clear();
                }
            }
            Some(Drag::Marquee { start, now, add }) => {
                let m = Rect::from_two_pos(start, now);
                if m.area() < 4.0 {
                    return;
                }
                let l = &self.layout;
                let inside: Vec<u64> = l
                    .objects
                    .iter()
                    .filter(|o| o.kind != 12 && visible(l, o))
                    .filter(|o| m.contains_rect(rect_of(&l.absolute(o))))
                    .map(|o| o.id)
                    .collect();
                if !add {
                    self.sel.clear();
                }
                self.sel.extend(inside);
            }
            Some(Drag::Create { start, now }) => self.create(start, now),
            None => {}
        }
    }

    /// Places a new object with the current tool over the dragged
    /// rectangle (a click gives the tool's default size).
    fn create(&mut self, start: Pos2, now: Pos2) {
        let tool = self.tool;
        self.tool = Tool::Select;
        let r0 = Rect::from_two_pos(start, now);
        let mut r = Rect::from_min_max(r0.min.round(), r0.max.round());
        let (w, h) = match tool {
            Tool::Text => (80.0, 20.0),
            Tool::Field => (160.0, 24.0),
            Tool::Button => (100.0, 30.0),
            Tool::Line => (100.0, 0.0),
            _ => (100.0, 60.0),
        };
        if r.width() < 3.0 && r.height() < 3.0 {
            r = Rect::from_min_size(start.round(), Vec2::new(w, h));
        }
        if tool == Tool::Line {
            // Mostly horizontal or vertical lines, like FileMaker's constrained drag.
            if r.height() < r.width() / 4.0 {
                r.max.y = r.min.y;
            } else if r.width() < r.height() / 4.0 {
                r.max.x = r.min.x;
            }
        }
        let rect = Bounds {
            top: r.top() as f64,
            left: r.left() as f64,
            bottom: r.bottom() as f64,
            right: r.right() as f64,
        };
        let base = LayoutObject {
            bounds: rect.clone(),
            ..Default::default()
        };
        match tool {
            Tool::Text => {
                self.checkpoint();
                let text = tr("Text").to_string();
                let id = self.add(LayoutObject {
                    kind: 2,
                    style_raw: new_style(70),
                    text: Some(text.clone()),
                    ..base
                });
                self.text_edit = Some((id, text));
            }
            Tool::Rect => {
                self.checkpoint();
                self.add(LayoutObject {
                    kind: 5,
                    style_raw: new_style(51),
                    ..base
                });
            }
            Tool::Line => {
                self.checkpoint();
                self.add(LayoutObject {
                    kind: 4,
                    style_raw: new_style(50),
                    ..base
                });
            }
            Tool::Field => {
                self.dialog = Some(Dialog::Field {
                    rect,
                    to: (self.layout.table_occurrence & 0xFFF) as u32,
                    field: None,
                    label: true,
                })
            }
            Tool::Button => {
                self.dialog = Some(Dialog::Button {
                    rect,
                    edit: None,
                    label: tr("Button").to_string(),
                    script: false,
                    target: self.layout.id,
                })
            }
            Tool::Select => {}
        }
    }

    /// The layout canvas.
    pub fn canvas(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        let l = self.layout.clone();
        let total = l.parts.iter().map(|p| p.top + p.height).fold(0.0, f64::max) as f32;
        let size = Vec2::new(RULER + GUTTER + l.width as f32 + 60.0, RULER + total + 60.0);
        let (resp, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
        let o0 = resp.rect.min + Vec2::new(RULER + GUTTER, RULER);
        let to_screen = |r: Rect| r.translate(o0.to_vec2());
        let page = Rect::from_min_size(o0, Vec2::new(l.width as f32, total));
        let theme = doc.db.themes.iter().find(|t| t.number == l.theme);
        // Outside the layout: grey, like FileMaker's pasteboard.
        painter.rect_filled(resp.rect, 0.0, Color32::from_gray(200));
        let bg = look::resolve(theme, &ObjectStyle::of_layout(&l))
            .background
            .unwrap_or(Color32::WHITE);
        painter.rect_filled(page, 0.0, bg);
        for p in &l.parts {
            let lk = look::resolve(theme, &ObjectStyle::parse(&p.style_raw));
            if let Some(c) = lk.background {
                let r = Rect::from_min_max(
                    egui::pos2(0.0, p.top as f32),
                    egui::pos2(l.width as f32, (p.top + p.height) as f32),
                );
                painter.rect_filled(to_screen(r), 0.0, c);
            }
        }
        // Grid.
        let grid = Stroke::new(0.5, Color32::from_rgba_unmultiplied(120, 140, 170, 40));
        let major = Stroke::new(0.5, Color32::from_rgba_unmultiplied(120, 140, 170, 90));
        let mut x = 0.0;
        while x <= l.width as f32 {
            let s = if (x / GRID) as i32 % 6 == 0 {
                major
            } else {
                grid
            };
            painter.vline(o0.x + x, page.y_range(), s);
            x += GRID;
        }
        let mut y = 0.0;
        while y <= total {
            let s = if (y / GRID) as i32 % 6 == 0 {
                major
            } else {
                grid
            };
            painter.hline(page.x_range(), o0.y + y, s);
            y += GRID;
        }
        // Objects, back to front.
        let layout_to = (l.table_occurrence & 0xFFF) as u32;
        let table = doc.db.table_of(layout_to).map(|t| t.id).unwrap_or(0);
        for o in l.objects.iter().rev() {
            if !visible(&l, o) || o.kind == 12 {
                continue;
            }
            let lk = look::resolve(theme, &ObjectStyle::parse(&o.style_raw));
            let r = to_screen(rect_of(&l.absolute(o)));
            paint_object(&painter, doc, &l, o, &lk, r, table, layout_to);
        }
        // Part boundaries and labels.
        let font = egui::FontId::proportional(11.0);
        for (i, p) in l.parts.iter().enumerate() {
            let yb = o0.y + (p.top + p.height) as f32;
            painter.hline(
                (o0.x - GUTTER)..=(o0.x + l.width as f32),
                yb,
                Stroke::new(1.0, Color32::from_gray(120)),
            );
            let label = Rect::from_min_max(
                egui::pos2(o0.x - GUTTER + 2.0, o0.y + p.top as f32 + 1.0),
                egui::pos2(
                    o0.x - 2.0,
                    (o0.y + p.top as f32 + 18.0)
                        .min(yb - 1.0)
                        .max(o0.y + p.top as f32 + 6.0),
                ),
            );
            let active = matches!(&self.drag, Some(Drag::Part { index, .. }) if *index == i);
            painter.rect_filled(
                label,
                3.0,
                if active {
                    Color32::from_rgb(200, 220, 250)
                } else {
                    Color32::from_gray(232)
                },
            );
            painter.rect_stroke(
                label,
                3.0,
                Stroke::new(0.5, Color32::from_gray(150)),
                StrokeKind::Inside,
            );
            painter.with_clip_rect(label).text(
                label.left_center() + Vec2::new(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                part_name(p.kind),
                font.clone(),
                Color32::from_gray(40),
            );
        }
        // Right edge of the layout.
        painter.vline(
            page.right(),
            page.y_range(),
            Stroke::new(1.0, Color32::from_gray(120)),
        );
        // Rulers (points).
        let ruler_bg = Color32::from_gray(245);
        let top_ruler = Rect::from_min_max(
            resp.rect.min,
            egui::pos2(resp.rect.max.x, resp.rect.min.y + RULER),
        );
        let left_ruler = Rect::from_min_max(
            resp.rect.min,
            egui::pos2(resp.rect.min.x + RULER, resp.rect.max.y),
        );
        painter.rect_filled(top_ruler, 0.0, ruler_bg);
        painter.rect_filled(left_ruler, 0.0, ruler_bg);
        let tick = Stroke::new(0.5, Color32::from_gray(110));
        let small = egui::FontId::proportional(9.0);
        let mut v = 0.0;
        while o0.x + v <= resp.rect.max.x {
            let h = if v as i32 % 100 == 0 {
                RULER
            } else if v as i32 % 50 == 0 {
                8.0
            } else {
                4.0
            };
            painter.vline(
                o0.x + v,
                (top_ruler.bottom() - h)..=top_ruler.bottom(),
                tick,
            );
            if v as i32 % 100 == 0 {
                painter.text(
                    egui::pos2(o0.x + v + 2.0, top_ruler.top()),
                    egui::Align2::LEFT_TOP,
                    format!("{v}"),
                    small.clone(),
                    Color32::from_gray(80),
                );
            }
            v += 10.0;
        }
        let mut v = 0.0;
        while o0.y + v <= resp.rect.max.y {
            let w = if v as i32 % 100 == 0 {
                RULER
            } else if v as i32 % 50 == 0 {
                8.0
            } else {
                4.0
            };
            painter.hline(
                (left_ruler.right() - w)..=left_ruler.right(),
                o0.y + v,
                tick,
            );
            if v as i32 % 100 == 0 && v > 0.0 {
                painter.text(
                    egui::pos2(left_ruler.left() + 1.0, o0.y + v + 1.0),
                    egui::Align2::LEFT_TOP,
                    format!("{v}"),
                    small.clone(),
                    Color32::from_gray(80),
                );
            }
            v += 10.0;
        }
        // Selection handles.
        let blue = Color32::from_rgb(30, 110, 230);
        for id in &self.sel {
            let Some(o) = l.objects.iter().find(|o| o.id == *id) else {
                continue;
            };
            let r = to_screen(rect_of(&l.absolute(o)));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, blue), StrokeKind::Outside);
            for h in handles(r) {
                let hr = Rect::from_center_size(h, Vec2::splat(HANDLE));
                painter.rect_filled(hr, 0.0, Color32::WHITE);
                painter.rect_stroke(hr, 0.0, Stroke::new(1.0, blue), StrokeKind::Inside);
            }
        }
        match &self.drag {
            Some(Drag::Marquee { start, now, .. }) => {
                let r = to_screen(Rect::from_two_pos(*start, *now));
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(30, 110, 230, 25));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, blue), StrokeKind::Inside);
            }
            Some(Drag::Create { start, now }) => {
                let r = to_screen(Rect::from_two_pos(*start, *now));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, Color32::from_gray(60)),
                    StrokeKind::Inside,
                );
            }
            _ => {}
        }

        // Interaction, in layout coordinates.
        let to_layout = |p: Pos2| (p - o0).to_pos2();
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.drag_started() || resp.clicked() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos());
            if let Some(p) = origin {
                self.press(to_layout(p), shift, p.x < o0.x);
            }
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.drag_to(to_layout(p));
        }
        if resp.drag_stopped() || resp.clicked() {
            self.release();
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && let Some(id) = self.hit(to_layout(p))
            && let Some(o) = self.object(id)
        {
            match o.kind {
                2 => self.text_edit = Some((id, o.text.clone().unwrap_or_default())),
                10 => {
                    let label = o
                        .label
                        .as_ref()
                        .map(|t| doc.db.eval_tokens(t, table, layout_to, None).to_text())
                        .unwrap_or_default();
                    let (script, target) = match o.button.as_ref().map(|s| &s.params) {
                        Some(fmp12::scripts::StepParams::PerformScript { script, .. }) => {
                            (true, script.unwrap_or(0))
                        }
                        Some(fmp12::scripts::StepParams::GoToLayout {
                            dest: fmp12::scripts::LayoutDest::Layout(t),
                            ..
                        }) => (false, *t),
                        _ => (false, l.id),
                    };
                    self.dialog = Some(Dialog::Button {
                        rect: o.bounds.clone(),
                        edit: Some(id),
                        label,
                        script,
                        target,
                    });
                }
                _ => {}
            }
        }
        // Cursor feedback.
        if let Some(p) = resp.hover_pos() {
            let lp = to_layout(p);
            if self.tool != Tool::Select {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            } else if let Some((_, h)) = self.handle_at(lp) {
                use egui::CursorIcon::*;
                let c = [
                    ResizeNorthWest,
                    ResizeVertical,
                    ResizeNorthEast,
                    ResizeHorizontal,
                ][h % 4];
                ui.ctx().set_cursor_icon(c);
            } else if self.boundary_at(lp.y).is_some() && (p.x < o0.x || self.hit(lp).is_none()) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }
        // Inline text editing.
        if let Some((id, buf)) = &mut self.text_edit {
            let id = *id;
            if let Some(o) = l.objects.iter().find(|o| o.id == id) {
                let r = to_screen(rect_of(&l.absolute(o)));
                let edit = ui.put(
                    r.expand(2.0),
                    egui::TextEdit::multiline(buf).desired_rows(1),
                );
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                if edit.lost_focus() {
                    self.end_text_edit();
                }
            } else {
                self.text_edit = None;
            }
        }
    }

    /// The Inspector: Position, Appearance and Data tabs.
    pub fn inspector(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        ui.horizontal(|ui| {
            for (t, name) in [
                (Tab::Position, "Position"),
                (Tab::Appearance, "Appearance"),
                (Tab::Data, "Data"),
            ] {
                ui.selectable_value(&mut self.tab, t, tr(name));
            }
        });
        ui.separator();
        let ids: Vec<u64> = self
            .layout
            .objects
            .iter()
            .filter(|o| self.sel.contains(&o.id))
            .map(|o| o.id)
            .collect();
        if ids.is_empty() {
            ui.label(tr("No objects selected"));
            if self.tab == Tab::Position {
                ui.horizontal(|ui| {
                    ui.label(tr("Layout width"));
                    let mut w = self.layout.width;
                    if ui
                        .add(egui::DragValue::new(&mut w).range(1.0..=32000.0))
                        .changed()
                    {
                        self.touch("width");
                        self.layout.width = w;
                    }
                });
            }
            return;
        }
        if ids.len() > 1 {
            ui.label(format!("{} {}", ids.len(), tr("objects selected")));
        }
        match self.tab {
            Tab::Position => self.position_tab(ui, ids[0], ids.len() == 1),
            Tab::Appearance => self.appearance_tab(ui, doc, &ids),
            Tab::Data => self.data_tab(ui, doc, &ids),
        }
    }

    fn position_tab(&mut self, ui: &mut egui::Ui, id: u64, single: bool) {
        let Some(o) = self.object(id).cloned() else {
            return;
        };
        if single {
            ui.horizontal(|ui| {
                ui.label(tr("Name"));
                let mut n = o.name.clone();
                if ui.text_edit_singleline(&mut n).changed() {
                    self.touch("name");
                    if let Some(x) = self.layout.objects.iter_mut().find(|x| x.id == id) {
                        x.name = n;
                    }
                }
            });
        }
        let abs = self.layout.absolute(&o);
        let mut v = [
            abs.left,
            abs.top,
            abs.right,
            abs.bottom,
            abs.right - abs.left,
            abs.bottom - abs.top,
        ];
        let before = v;
        egui::Grid::new("position").num_columns(4).show(ui, |ui| {
            for row in [[0usize, 1], [2, 3], [4, 5]] {
                for i in row {
                    ui.label(tr(["Left", "Top", "Right", "Bottom", "Width", "Height"][i]));
                    ui.add(egui::DragValue::new(&mut v[i]).speed(1.0).max_decimals(1));
                }
                ui.end_row();
            }
        });
        if v != before {
            self.touch("position");
            let mut b = abs.clone();
            if v[0] != before[0] {
                b.right += v[0] - b.left;
                b.left = v[0];
            }
            if v[1] != before[1] {
                b.bottom += v[1] - b.top;
                b.top = v[1];
            }
            if v[2] != before[2] {
                b.right = v[2].max(b.left);
            }
            if v[3] != before[3] {
                b.bottom = v[3].max(b.top);
            }
            if v[4] != before[4] {
                b.right = b.left + v[4].max(0.0);
            }
            if v[5] != before[5] {
                b.bottom = b.top + v[5].max(0.0);
            }
            // Back to coordinates relative to the container.
            let (dy, dx) = (abs.top - o.bounds.top, abs.left - o.bounds.left);
            if let Some(x) = self.layout.objects.iter_mut().find(|x| x.id == id) {
                x.bounds = Bounds {
                    top: b.top - dy,
                    left: b.left - dx,
                    bottom: b.bottom - dy,
                    right: b.right - dx,
                };
            }
        }
    }

    fn set_styles(&mut self, ids: &[u64], what: &'static str, f: impl Fn(&[u8]) -> Vec<u8>) {
        self.touch(what);
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| ids.contains(&o.id))
        {
            o.style_raw = f(&o.style_raw);
        }
    }

    fn appearance_tab(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        let theme = doc.db.themes.iter().find(|t| t.number == self.layout.theme);
        let style = ObjectStyle::parse(&o.style_raw);
        // Named styles of the object's type (shapes share type 54).
        let ty = style.object_type;
        let mut names: Vec<String> = theme
            .map(|t| {
                t.styles
                    .keys()
                    .filter(|(t2, n)| {
                        !n.is_empty() && (*t2 == ty || matches!(ty, 50..=53) && *t2 == 54)
                    })
                    .map(|(_, n)| n.clone())
                    .collect()
            })
            .unwrap_or_default();
        names.dedup();
        let shown = |n: &str| -> String {
            if n.is_empty() {
                tr("Default").to_string()
            } else {
                self.displays
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| n.to_string())
            }
        };
        ui.label(tr("Style"));
        let mut pick = None;
        egui::ComboBox::from_id_salt("named_style")
            .selected_text(shown(&style.name))
            .width(220.0)
            .show_ui(ui, |ui| {
                for n in std::iter::once(String::new()).chain(names.iter().cloned()) {
                    if ui.selectable_label(n == style.name, shown(&n)).clicked() {
                        pick = Some(n);
                    }
                }
            });
        if let Some(n) = pick {
            let d = self.displays.get(&n).cloned().unwrap_or_else(|| n.clone());
            self.set_styles(ids, "style", |s| theme::with_style_name(s, &n, &d));
        }
        ui.separator();
        let lk = look::resolve(theme, &style);
        let local = style
            .local
            .get(&(SELF, NORMAL))
            .cloned()
            .unwrap_or_default();
        let local_text = style
            .local
            .get(&(TEXT, NORMAL))
            .cloned()
            .unwrap_or_default();
        use fmp12::theme::PropsExt;
        egui::Grid::new("appearance").num_columns(3).show(ui, |ui| {
            // Fill.
            ui.label(tr("Fill"));
            let mut c = lk.background.unwrap_or(Color32::TRANSPARENT);
            if ui.color_edit_button_srgba(&mut c).changed() {
                let v = theme::Value::Color(rgba(c));
                self.set_styles(ids, "fill", |s| {
                    theme::with_local_prop(s, SELF, NORMAL, prop::BACKGROUND_COLOR, Some(&v))
                });
            }
            let has = local.color(prop::BACKGROUND_COLOR).is_some();
            if ui
                .add_enabled(has, egui::Button::new("×"))
                .on_hover_text(tr("Use the style's value"))
                .clicked()
            {
                self.set_styles(ids, "fill-reset", |s| {
                    theme::with_local_prop(s, SELF, NORMAL, prop::BACKGROUND_COLOR, None)
                });
            }
            ui.end_row();
            // Border: colour and width on all four sides.
            ui.label(tr("Border"));
            let (w0, c0) = lk.borders[0];
            let mut c = c0;
            let mut w = w0 as f64;
            let cr = ui.color_edit_button_srgba(&mut c);
            let wr = ui.add(
                egui::DragValue::new(&mut w)
                    .range(0.0..=20.0)
                    .speed(0.25)
                    .suffix(" pt"),
            );
            if cr.changed() || wr.changed() {
                let (cv, wv) = (theme::Value::Color(rgba(c)), pt(w));
                self.set_styles(ids, "border", |s| {
                    let mut s = s.to_vec();
                    for i in 0..4 {
                        s = theme::with_local_prop(
                            &s,
                            SELF,
                            NORMAL,
                            prop::BORDER_COLOR + i,
                            Some(&cv),
                        );
                        s = theme::with_local_prop(
                            &s,
                            SELF,
                            NORMAL,
                            prop::BORDER_STYLE + i,
                            Some(&theme::Value::Enum(if w > 0.0 { 5 } else { 1 })),
                        );
                        s = theme::with_local_prop(
                            &s,
                            SELF,
                            NORMAL,
                            prop::BORDER_WIDTH + i,
                            Some(&wv),
                        );
                    }
                    s
                });
            }
            ui.end_row();
            // Text colour (the `.text` element, as FileMaker's local styles).
            ui.label(tr("Text color"));
            let mut c = lk.color;
            if ui.color_edit_button_srgba(&mut c).changed() {
                let v = theme::Value::Color(rgba(c));
                self.set_styles(ids, "text", |s| {
                    theme::with_local_prop(s, TEXT, NORMAL, prop::COLOR, Some(&v))
                });
            }
            let has = local_text.color(prop::COLOR).is_some();
            if ui
                .add_enabled(has, egui::Button::new("×"))
                .on_hover_text(tr("Use the style's value"))
                .clicked()
            {
                self.set_styles(ids, "text-reset", |s| {
                    theme::with_local_prop(s, TEXT, NORMAL, prop::COLOR, None)
                });
            }
            ui.end_row();
        });
    }

    fn data_tab(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        let Some((to_ref, field, rep)) = o.field else {
            ui.label(tr("No field selected"));
            return;
        };
        let to = (to_ref & 0xFFF) as u32;
        ui.label(tr("Field"));
        let mut new_to = to;
        egui::ComboBox::from_id_salt("data_to")
            .selected_text(
                doc.db
                    .occurrences
                    .iter()
                    .find(|t| t.id == to)
                    .map_or("?", |t| t.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for t in &doc.db.occurrences {
                    ui.selectable_value(&mut new_to, t.id, &t.name);
                }
            });
        let fields: Vec<(u32, String)> = doc
            .db
            .table_of(new_to)
            .map(|t| t.fields.iter().map(|f| (f.id, f.name.clone())).collect())
            .unwrap_or_default();
        let mut new_field = field as u32;
        egui::ComboBox::from_id_salt("data_field")
            .selected_text(
                fields
                    .iter()
                    .find(|f| f.0 == new_field && new_to == to)
                    .map_or("", |f| f.1.as_str()),
            )
            .show_ui(ui, |ui| {
                for (id, name) in &fields {
                    ui.selectable_value(&mut new_field, *id, name);
                }
            });
        if (new_to, new_field) != (to, field as u32) && fields.iter().any(|f| f.0 == new_field) {
            self.touch("field");
            if let Some(x) = self.layout.objects.iter_mut().find(|x| x.id == o.id) {
                x.field = Some((0x104000 | new_to as u64, new_field as u64, rep));
            }
        }
        ui.separator();
        ui.label(tr("Control style"));
        let mut d = o.display.min(3);
        egui::ComboBox::from_id_salt("data_display")
            .selected_text(if o.display <= 3 {
                tr(DISPLAY_NAMES[o.display as usize])
            } else {
                "…"
            })
            .show_ui(ui, |ui| {
                for (i, n) in DISPLAY_NAMES.iter().enumerate() {
                    ui.selectable_value(&mut d, i as u64, tr(n));
                }
            });
        if d != o.display.min(3) || (o.display > 3 && d != 3) {
            self.touch("display");
            for x in self
                .layout
                .objects
                .iter_mut()
                .filter(|x| ids.contains(&x.id) && x.kind == 1)
            {
                x.display = d;
                x.style_raw = theme::with_object_type(&x.style_raw, DISPLAY_TYPES[d as usize]);
            }
        }
        ui.label(tr("Value list"));
        let mut vl = o.value_list;
        let name = |v: Option<u32>| {
            v.and_then(|v| doc.db.value_lists.iter().find(|x| x.id == v))
                .map_or(tr("<None>").to_string(), |x| x.name.clone())
        };
        egui::ComboBox::from_id_salt("data_vl")
            .selected_text(name(vl))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut vl, None, tr("<None>"));
                for v in &doc.db.value_lists {
                    ui.selectable_value(&mut vl, Some(v.id), &v.name);
                }
            });
        if vl != o.value_list {
            self.touch("valuelist");
            for x in self
                .layout
                .objects
                .iter_mut()
                .filter(|x| ids.contains(&x.id) && x.kind == 1)
            {
                x.value_list = vl;
            }
        }
    }

    /// Specify Field, Button Setup and Layout Setup dialogs.
    pub fn dialogs(&mut self, ctx: &egui::Context, doc: &Doc, scripts: &[(u32, String)]) {
        let mut close = false;
        let mut done: Option<Dialog> = None;
        match &mut self.dialog {
            Some(Dialog::Field {
                to, field, label, ..
            }) => {
                egui::Modal::new(egui::Id::NULL.with("specify_field")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Specify Field"));
                    egui::ComboBox::from_id_salt("sf_to")
                        .selected_text(
                            doc.db
                                .occurrences
                                .iter()
                                .find(|t| t.id == *to)
                                .map_or("?", |t| t.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for t in &doc.db.occurrences {
                                if ui.selectable_value(to, t.id, &t.name).clicked() {
                                    *field = None;
                                }
                            }
                        });
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for f in doc
                                .db
                                .table_of(*to)
                                .map(|t| t.fields.as_slice())
                                .unwrap_or_default()
                            {
                                let r = ui.selectable_label(*field == Some(f.id), &f.name);
                                if r.clicked() {
                                    *field = Some(f.id);
                                }
                                if r.double_clicked() {
                                    done = Some(Dialog::Field {
                                        rect: Bounds::default(),
                                        to: *to,
                                        field: *field,
                                        label: *label,
                                    });
                                }
                            }
                        });
                    ui.checkbox(label, tr("Create label"));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(field.is_some(), egui::Button::new("OK"))
                            .clicked()
                        {
                            done = Some(Dialog::Field {
                                rect: Bounds::default(),
                                to: *to,
                                field: *field,
                                label: *label,
                            });
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Some(Dialog::Button {
                label,
                script,
                target,
                ..
            }) => {
                egui::Modal::new(egui::Id::NULL.with("button_setup")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Button Setup"));
                    ui.horizontal(|ui| {
                        ui.label(tr("Label"));
                        ui.text_edit_singleline(label);
                    });
                    ui.horizontal(|ui| {
                        if ui.radio(!*script, tr("Go to Layout")).clicked() && *script {
                            *script = false;
                            *target = doc.layouts.first().map_or(0, |l| l.id);
                        }
                        if ui.radio(*script, tr("Perform Script")).clicked() && !*script {
                            *script = true;
                            *target = scripts.first().map_or(0, |s| s.0);
                        }
                    });
                    let items: Vec<(u32, String)> = if *script {
                        scripts.to_vec()
                    } else {
                        doc.layouts.iter().map(|l| (l.id, l.name.clone())).collect()
                    };
                    egui::ComboBox::from_id_salt("bs_target")
                        .selected_text(
                            items
                                .iter()
                                .find(|i| i.0 == *target)
                                .map_or("", |i| i.1.as_str()),
                        )
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for (id, n) in &items {
                                ui.selectable_value(target, *id, n);
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                items.iter().any(|i| i.0 == *target),
                                egui::Button::new("OK"),
                            )
                            .clicked()
                        {
                            done = Some(Dialog::Button {
                                rect: Bounds::default(),
                                edit: None,
                                label: label.clone(),
                                script: *script,
                                target: *target,
                            });
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            }
            None => {}
        }
        if let Some(d) = done {
            let open = self.dialog.take();
            self.finish_dialog(open, d, doc);
        } else if close {
            self.dialog = None;
        }
        // Layout Setup.
        let mut setup_done = None;
        if let Some((name, to)) = &mut self.setup {
            egui::Modal::new(egui::Id::NULL.with("layout_setup")).show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.heading(tr("Layout Setup"));
                ui.horizontal(|ui| {
                    ui.label(tr("Layout Name"));
                    ui.text_edit_singleline(name);
                });
                ui.label(tr("Show records from"));
                let cur = (*to & 0xFFF) as u32;
                egui::ComboBox::from_id_salt("setup_to")
                    .selected_text(
                        doc.db
                            .occurrences
                            .iter()
                            .find(|t| t.id == cur)
                            .map_or("?", |t| t.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for t in &doc.db.occurrences {
                            if ui.selectable_label(t.id == cur, &t.name).clicked() {
                                *to = 0x104000 | t.id as u64;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!name.trim().is_empty(), egui::Button::new("OK"))
                        .clicked()
                    {
                        setup_done = Some(Some((name.trim().to_string(), *to)));
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        setup_done = Some(None);
                    }
                });
            });
        }
        if let Some(r) = setup_done {
            self.setup = None;
            if let Some((name, to)) = r
                && (name != self.layout.name || to != self.layout.table_occurrence)
            {
                self.checkpoint();
                self.layout.name = name;
                self.layout.table_occurrence = to;
            }
        }
    }

    fn finish_dialog(&mut self, open: Option<Dialog>, answer: Dialog, doc: &Doc) {
        match (open, answer) {
            (
                Some(Dialog::Field { rect, .. }),
                Dialog::Field {
                    to,
                    field: Some(f),
                    label,
                    ..
                },
            ) => {
                self.checkpoint();
                if label {
                    let name = doc
                        .db
                        .table_of(to)
                        .and_then(|t| t.field(f))
                        .map_or(String::new(), |x| x.name.clone());
                    let w = 110.0;
                    let lb = Bounds {
                        top: rect.top,
                        left: (rect.left - w - 6.0).max(0.0),
                        bottom: rect.bottom,
                        right: (rect.left - 6.0).max(w.min(rect.left)),
                    };
                    self.add(LayoutObject {
                        kind: 2,
                        bounds: lb,
                        style_raw: new_style(70),
                        text: Some(name),
                        ..Default::default()
                    });
                }
                let id = self.add(LayoutObject {
                    kind: 1,
                    bounds: rect,
                    style_raw: new_style(61),
                    field: Some((0x104000 | to as u64, f as u64, 1)),
                    ..Default::default()
                });
                if label {
                    self.sel.insert(id);
                }
            }
            (
                Some(Dialog::Button { rect, edit, .. }),
                Dialog::Button {
                    label,
                    script,
                    target,
                    ..
                },
            ) => {
                let step_id = if script {
                    fmp12::scripts::id::PERFORM_SCRIPT
                } else {
                    fmp12::scripts::id::GO_TO_LAYOUT
                };
                let step = fmp12::scripts::targeted_step(step_id, target);
                let tokens = fmp12::save::text_literal_tokens(&label);
                self.checkpoint();
                match edit.and_then(|id| self.layout.objects.iter_mut().find(|o| o.id == id)) {
                    Some(o) => {
                        if o.button.as_ref().map(|s| (s.id, &s.params))
                            != Some((step.id, &step.params))
                        {
                            o.button = Some(step);
                        }
                        o.label = Some(tokens);
                    }
                    None => {
                        self.add(LayoutObject {
                            kind: 10,
                            bounds: rect,
                            style_raw: new_style(80),
                            label: Some(tokens),
                            button: Some(step),
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Paints one object as Layout mode shows it: theme look, field names in
/// fields, labels on buttons, static text verbatim.
#[allow(clippy::too_many_arguments)]
fn paint_object(
    painter: &egui::Painter,
    doc: &Doc,
    l: &Layout,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    table: u32,
    layout_to: u32,
) {
    let thin = Stroke::new(0.5, Color32::from_gray(150));
    match o.kind {
        // Line: from the top-left to the bottom-right corner.
        4 => {
            let (w, c) = lk.borders[0];
            let s = if w > 0.0 {
                Stroke::new(w, c)
            } else {
                Stroke::new(1.0, Color32::from_gray(60))
            };
            painter.line_segment([r.left_top(), r.right_bottom()], s);
            return;
        }
        _ => lk.paint_box(painter, r),
    }
    let boxed = lk.background.is_some() || lk.borders.iter().any(|b| b.0 > 0.0);
    match o.kind {
        1 => {
            if !boxed {
                painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            }
            let name = o
                .field
                .and_then(|(to, f, _)| {
                    let to = (to & 0xFFF) as u32;
                    let n = doc.db.table_of(to)?.field(f as u32)?.name.clone();
                    Some(if to == layout_to { n } else { format!("::{n}") })
                })
                .unwrap_or_else(|| "<?>".into());
            lk.paint_text(painter, r, &name);
            // Control styles: an arrow for lists and menus, a box for checkboxes.
            let c = lk.content_rect(r);
            match o.display {
                1 | 2 => {
                    let x = c.right() - 6.0;
                    let y = c.center().y;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x - 4.0, y - 2.0),
                            egui::pos2(x + 4.0, y - 2.0),
                            egui::pos2(x, y + 3.0),
                        ],
                        Color32::from_gray(90),
                        Stroke::NONE,
                    ));
                }
                3 => {
                    let b = Rect::from_min_size(
                        c.left_center() - Vec2::new(0.0, 5.0),
                        Vec2::splat(10.0),
                    );
                    painter.rect_stroke(
                        b,
                        1.0,
                        Stroke::new(1.0, Color32::from_gray(90)),
                        StrokeKind::Inside,
                    );
                }
                _ => {}
            }
        }
        2 => lk.paint_text(painter, r, o.text.as_deref().unwrap_or("")),
        5 if !boxed => {
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
        }
        10 => {
            if let Some(t) = &o.label {
                let text = doc.db.eval_tokens(t, table, layout_to, None).to_text();
                lk.paint_text(painter, r, &text);
            } else if let Some(t) = &o.text {
                lk.paint_text(painter, r, t);
            }
            if !boxed {
                painter.rect_stroke(r, 4.0, thin, StrokeKind::Inside);
            }
        }
        // Portal: rows; tab control: tabs.
        9 => {
            let rows = o.portal.map_or(1, |p| p.1.max(1));
            let h = r.height() / rows as f32;
            for i in 1..rows {
                painter.hline(r.x_range(), r.top() + h * i as f32, thin);
            }
            if !boxed {
                painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            }
        }
        11 => {
            let front = front_panel(l, o);
            let mut panels: Vec<&LayoutObject> = l
                .objects
                .iter()
                .filter(|p| p.parent == Some(o.id))
                .collect();
            panels.sort_by_key(|p| p.id);
            let mut x = r.left();
            for p in panels {
                let label = p.text.clone().unwrap_or_default();
                let w = (label.chars().count() as f32 * 8.0 + 32.0).max(60.0);
                let tab = Rect::from_min_size(egui::pos2(x, r.top()), Vec2::new(w, 26.0));
                let on = front == Some(p.id);
                painter.rect_filled(tab, 0.0, Color32::from_gray(if on { 255 } else { 232 }));
                painter.rect_stroke(tab, 0.0, thin, StrokeKind::Inside);
                painter.text(
                    tab.center(),
                    egui::Align2::CENTER_CENTER,
                    &label,
                    lk.font(),
                    Color32::from_gray(40),
                );
                x += w;
            }
        }
        _ if !boxed => {
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
        }
        _ => {}
    }
}

/// A new blank layout: header, body and footer, as FileMaker's "Blank"
/// layout, showing records of `base`'s table occurrence with its theme.
pub fn blank_layout(base: &Layout, id: u32, name: String) -> Layout {
    let part = |kind, top, height, ty| Part {
        kind,
        top,
        height,
        style: String::new(),
        style_raw: vec![0x18, ty],
    };
    Layout {
        id,
        name,
        table_occurrence: base.table_occurrence,
        width: base.width.max(320.0),
        theme: base.theme,
        style_raw: base.style_raw.clone(),
        css_raw: base.css_raw.clone(),
        parts: vec![
            part(1, 0.0, 72.0, 32),
            part(4, 72.0, 240.0, 34),
            part(7, 312.0, 48.0, 38),
        ],
        objects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contacts (en) from a local FileMaker install, if any.
    fn doc() -> Option<Doc> {
        let p = std::path::Path::new(
            r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\Contacts\en.fmp12",
        );
        let file = fmp12::File::open(p).ok()?;
        let db = engine::Database::load(&file).ok()?;
        let layouts = db
            .layouts
            .iter()
            .filter(|l| l.table_occurrence != 0)
            .cloned()
            .collect();
        Some(Doc {
            name: "Contacts".into(),
            db,
            layouts,
            file,
            path: None,
        })
    }

    fn bounds(ds: &Designer, id: u64) -> (f64, f64, f64, f64) {
        let b = &ds.object(id).unwrap().bounds;
        (b.top, b.left, b.bottom, b.right)
    }

    #[test]
    fn edit_with_the_mouse_and_save() {
        let Some(mut d) = doc() else { return };
        let i = d
            .layouts
            .iter()
            .position(|l| l.name == "Contact List")
            .unwrap();
        let mut ds = Designer::new(&d, i);
        // Field 9 (Last Name) spans (112, 366)–(156, 632): click and drag it.
        let p = egui::pos2(400.0, 130.0);
        ds.press(p, false, false);
        assert_eq!(ds.sel, BTreeSet::from([9]));
        ds.drag_to(p + Vec2::new(10.0, 20.0));
        ds.release();
        assert_eq!(bounds(&ds, 9), (132.0, 376.0, 176.0, 642.0));
        // Arrow keys: Shift moves by 10.
        ds.nudge(10.0, 0.0);
        assert_eq!(bounds(&ds, 9).1, 386.0);
        ds.undo();
        assert_eq!(bounds(&ds, 9).1, 376.0);
        ds.undo();
        assert_eq!(bounds(&ds, 9), (112.0, 366.0, 156.0, 632.0));
        ds.redo();
        assert_eq!(bounds(&ds, 9).0, 132.0);
        // Resize from the bottom-right handle.
        let br = egui::pos2(642.0, 176.0);
        ds.press(br, false, false);
        ds.drag_to(br + Vec2::new(-42.0, 4.0));
        ds.release();
        assert_eq!(bounds(&ds, 9), (132.0, 376.0, 180.0, 600.0));
        // Shift-click adds field 8, marquee selects fields 10 and 12.
        ds.press(egui::pos2(700.0, 120.0), true, false);
        ds.release();
        assert_eq!(ds.sel, BTreeSet::from([8, 9]));
        ds.press(egui::pos2(5.0, 108.0), false, false);
        ds.drag_to(egui::pos2(365.0, 160.0));
        ds.release();
        assert!(ds.sel.contains(&10) && ds.sel.contains(&12) && !ds.sel.contains(&9));
        // Body (104–164) grows by 20 from its boundary in the part gutter.
        ds.sel.clear();
        ds.press(egui::pos2(-10.0, 164.0), false, true);
        ds.drag_to(egui::pos2(-10.0, 184.0));
        ds.release();
        let body = ds.layout.parts.iter().find(|p| p.kind == 4).unwrap();
        assert_eq!(body.height, 80.0);
        // A rectangle drawn with the tool, deleted, then restored by undo.
        ds.tool = Tool::Rect;
        ds.press(egui::pos2(700.0, 50.0), false, false);
        ds.drag_to(egui::pos2(760.0, 70.0));
        ds.release();
        let r = *ds.sel.iter().next().unwrap();
        assert_eq!(ds.object(r).unwrap().kind, 5);
        assert_eq!(bounds(&ds, r), (50.0, 700.0, 70.0, 760.0));
        ds.delete_selection();
        assert!(ds.object(r).is_none());
        ds.undo();
        assert!(ds.object(r).is_some());
        // Save into the document, then to the file, and read it back.
        assert!(ds.changed());
        ds.apply(&mut d);
        assert!(!ds.changed());
        let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
        let f = fmp12::File::from_bytes(bytes).unwrap();
        let back = fmp12::layouts::layouts(&f).unwrap();
        let l = back.iter().find(|l| l.name == "Contact List").unwrap();
        let o9 = l.objects.iter().find(|o| o.id == 9).unwrap();
        assert_eq!(o9.bounds, ds.object(9).unwrap().bounds);
        assert_eq!(l.parts, ds.layout.parts);
        assert_eq!(l.objects.iter().find(|o| o.id == r).unwrap().kind, 5);
    }
}
