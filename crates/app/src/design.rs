//! Layout mode (M6): the layout canvas with rulers, grid, guides and part
//! labels, the tools, selection, move and resize with snapping, the Arrange
//! commands (align, distribute, resize, z-order, group, lock), the
//! clipboard, parts, undo/redo, the Inspector (Position, Styles,
//! Appearance, Data) and the dialogs that place fields, portals, tab
//! controls and buttons. The Layouts menu (new, duplicate, delete) writes
//! to the file through `fmp12::schema_edit` ([`run_layout_op`]).
//!
//! The designer edits a copy of the layout. FileMaker asks whether to save
//! when the user leaves the layout; "Save" copies the edit into the open
//! document, and File › Save writes it to the `.fmp12` through
//! `fmp12::save::save_layouts`.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use fmp12::layouts::{Bounds, Layout, LayoutObject, Part, anchor, field_flags, part};
use fmp12::theme::{self, Length, NORMAL, ObjectStyle, PropsExt, Rgba, SELF, TEXT, Unit, prop};

use crate::Doc;
use crate::i18n::tr;
use crate::look::{self, Look};

#[cfg(test)]
mod fms_corpus;
pub(crate) mod objects;

const RULER: f32 = 18.0;
/// Left margin holding the part labels.
const GUTTER: f32 = 96.0;
/// Grid spacing (points), FileMaker's default major grid / 6.
const GRID: f32 = 12.0;
const HANDLE: f32 = 6.0;
/// Distance (points) within which edges snap to guides.
const SNAP: f64 = 5.0;

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

/// Field control styles: display value (object field 8.5), style type.
/// Radio button sets (4, 65) were checked in FileMaker's XML; a concealed
/// edit box is an edit box with ield_flags::CONCEALED.
pub(crate) const CONTROLS: [(u64, u32, &str); 7] = [
    (0, 61, "Edit box"),
    (1, 62, "Drop-down list"),
    (2, 63, "Pop-up menu"),
    (3, 64, "Checkbox set"),
    (4, 65, "Radio button set"),
    (6, 66, "Drop-down calendar"),
    (0, 61, "Concealed edit box"),
];

/// Part kinds the Part Setup dialog adds, in FileMaker's order.
const PART_KINDS: [u64; 8] = [
    part::TITLE_HEADER,
    part::HEADER,
    part::LEADING_GRAND_SUMMARY,
    part::LEADING_SUB_SUMMARY,
    part::TRAILING_SUB_SUMMARY,
    part::TRAILING_GRAND_SUMMARY,
    part::FOOTER,
    part::TITLE_FOOTER,
];

/// Object kinds (field 1). Oval 7 and group 8 were checked in FileMaker's
/// XML (FORMAT.md §10).
mod kind {
    pub const FIELD: u64 = 1;
    pub const TEXT: u64 = 2;
    pub const LINE: u64 = 4;
    pub const RECT: u64 = 5;
    pub const OVAL: u64 = 7;
    pub const GROUP: u64 = 8;
    pub const PORTAL: u64 = 9;
    pub const BUTTON: u64 = 10;
    pub const TAB_CONTROL: u64 = 11;
    pub const TAB_PANEL: u64 = 12;
    pub const SLIDE_PANEL: u64 = 17;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Text,
    Field,
    Rect,
    Oval,
    Line,
    Button,
    Portal,
    TabControl,
    PopoverButton,
    SlideControl,
    ButtonBar,
    WebViewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Position,
    Styles,
    Appearance,
    Data,
}

/// Layouts menu commands, run on the file by [`run_layout_op`].
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutOp {
    New { name: String, to: u32 },
    Duplicate,
    Delete,
}

/// What to do once the "save changes?" question is answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    Exit,
    Switch(usize),
    Op(LayoutOp),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

/// A guide dragged from a ruler (layout coordinates, points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub at: f64,
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
    Guide(usize),
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
    /// Portal Setup: related occurrence, rows, fields to place in the row.
    Portal {
        rect: Bounds,
        to: u32,
        rows: u32,
        fields: Vec<u32>,
    },
    /// Tab Control Setup: one tab name per line.
    TabControl {
        rect: Bounds,
        tabs: String,
    },
    /// Part Setup: kind, and the break field of a sub-summary.
    Part {
        kind: u64,
        to: u32,
        field: Option<u32>,
    },
    NewLayout {
        name: String,
        to: u32,
    },
    DeleteLayout,
}

/// Layout objects copied (Edit › Copy), shared by every layout.
static CLIPBOARD: std::sync::Mutex<Vec<LayoutObject>> = std::sync::Mutex::new(Vec::new());

pub struct Designer {
    /// Index of the layout in `Doc::layouts`.
    pub index: usize,
    pub layout: Layout,
    saved: Layout,
    sel: BTreeSet<u64>,
    /// Part selected by a click on its label.
    sel_part: Option<usize>,
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    tool: Tool,
    drag: Option<Drag>,
    tab: Tab,
    text_edit: Option<(u64, String)>,
    dialog: Option<Dialog>,
    /// Layout Setup dialog (`layout_setup.rs`).
    setup: Option<crate::layout_setup::Setup>,
    /// Pending "save changes?" question.
    pub ask: Option<Then>,
    /// A request for the app: leave the layout, or run a Layouts command.
    pub pending: Option<Then>,
    /// Inspector property last edited: consecutive edits make one undo step.
    touched: Option<&'static str>,
    /// Display names of the named styles seen in the file (CSS name → name).
    displays: BTreeMap<String, String>,
    /// Fonts of the file's font catalog: (index = id − 128, family).
    fonts: Vec<(i64, String)>,
    next_id: u64,
    pub show_grid: bool,
    pub snap_grid: bool,
    pub show_rulers: bool,
    pub guides: Vec<Guide>,
    /// Screen position of the layout's (0, 0) in the last frame.
    pub canvas_origin: Pos2,
    /// Conditional formatting, tab order, web viewer dialogs (`objects.rs`).
    extra: objects::Extra,
}

fn rect_of(b: &Bounds) -> Rect {
    Rect::from_min_max(
        egui::pos2(b.left as f32, b.top as f32),
        egui::pos2(b.right as f32, b.bottom as f32),
    )
}

fn shift(b: &mut Bounds, dx: f64, dy: f64) {
    b.top += dy;
    b.bottom += dy;
    b.left += dx;
    b.right += dx;
}

fn union(bs: impl IntoIterator<Item = Bounds>) -> Option<Bounds> {
    bs.into_iter().reduce(|a, b| Bounds {
        top: a.top.min(b.top),
        left: a.left.min(b.left),
        bottom: a.bottom.max(b.bottom),
        right: a.right.max(b.right),
    })
}

pub fn part_name(kind: u64) -> &'static str {
    tr(match kind {
        1 => "Header",
        2 => "Leading Grand Summary",
        3 => "Sub-summary",
        4 => "Body",
        5 => "Trailing Sub-summary",
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

/// `id` and everything inside it.
fn subtree(l: &Layout, id: u64) -> BTreeSet<u64> {
    l.objects
        .iter()
        .filter(|o| o.id == id || ancestors(l, o).contains(&id))
        .map(|o| o.id)
        .collect()
}

/// Front panel of a tab control: field 6, else its first panel.
fn front_panel(l: &Layout, tc: &LayoutObject) -> Option<u64> {
    tc.active_child.or_else(|| {
        l.objects
            .iter()
            .filter(|x| {
                x.parent == Some(tc.id) && matches!(x.kind, kind::TAB_PANEL | kind::SLIDE_PANEL)
            })
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
        if !matches!(p.kind, kind::TAB_PANEL | kind::SLIDE_PANEL) {
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

/// Snaps `v` to the nearest of `lines` within [`SNAP`]; the offset to add.
fn snap_to(v: f64, lines: &[f64]) -> Option<f64> {
    lines
        .iter()
        .map(|g| g - v)
        .filter(|d| d.abs() <= SNAP)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
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
        let fonts = fmp12::theme::fonts(&doc.file)
            .unwrap_or_default()
            .into_iter()
            .filter(|(id, _)| *id >= 128)
            .map(|(id, f)| (id as i64 - 128, f))
            .collect();
        let next_id = layout.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
        Designer {
            index,
            saved: layout.clone(),
            layout,
            sel: BTreeSet::new(),
            sel_part: None,
            undo: Vec::new(),
            redo: Vec::new(),
            tool: Tool::Select,
            drag: None,
            tab: Tab::Position,
            text_edit: None,
            dialog: None,
            setup: None,
            ask: None,
            pending: None,
            touched: None,
            displays,
            fonts,
            next_id,
            show_grid: true,
            snap_grid: false,
            show_rulers: true,
            guides: Vec::new(),
            canvas_origin: Pos2::ZERO,
            extra: Default::default(),
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
        doc.triggers
            .layouts
            .insert(self.layout.id, self.layout.triggers.clone());
        match doc.db.layouts.iter_mut().find(|l| l.id == self.layout.id) {
            Some(l) => *l = self.layout.clone(),
            None => doc.db.layouts.push(self.layout.clone()),
        }
        self.saved = self.layout.clone();
    }

    /// After the file was saved: the layout as re-read (new parts get
    /// their node keys). Edits made since stay.
    pub fn reloaded(&mut self, doc: &Doc) {
        if let Some(l) = doc.layouts.iter().position(|l| l.id == self.layout.id) {
            self.index = l;
            self.layout = doc.layouts[l].clone();
            self.saved = self.layout.clone();
            self.undo.clear();
            self.redo.clear();
            self.prune_selection();
        }
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
        if self.sel_part.is_some_and(|p| p >= self.layout.parts.len()) {
            self.sel_part = None;
        }
    }

    pub fn select_all(&mut self) {
        self.sel_part = None;
        self.sel = self
            .layout
            .objects
            .iter()
            .filter(|o| o.parent.is_none())
            .map(|o| o.id)
            .collect();
    }

    /// Selects `ids` (the UI tests).
    #[cfg(test)]
    pub fn select(&mut self, ids: impl IntoIterator<Item = u64>) {
        self.sel = ids.into_iter().collect();
        self.sel_part = None;
    }

    #[cfg(test)]
    pub fn selection(&self) -> Vec<u64> {
        self.sel.iter().copied().collect()
    }

    pub fn has_selection(&self) -> bool {
        !self.sel.is_empty()
    }

    fn locked(o: &LayoutObject) -> bool {
        o.options & fmp12::layouts::LOCKED != 0
    }

    /// Deletes the selected objects and everything inside them (locked
    /// objects stay), or the selected part.
    pub fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            if let Some(p) = self.sel_part {
                self.delete_part(p);
            }
            return;
        }
        let l = &self.layout;
        let picked: Vec<u64> = l
            .objects
            .iter()
            .filter(|o| self.sel.contains(&o.id) && !Self::locked(o))
            .map(|o| o.id)
            .collect();
        if picked.is_empty() {
            return;
        }
        let gone: BTreeSet<u64> = picked.iter().flat_map(|id| subtree(l, *id)).collect();
        self.checkpoint();
        self.layout.objects.retain(|o| !gone.contains(&o.id));
        self.sel.retain(|i| !gone.contains(i));
    }

    /// Selected objects not inside another selected object: moving a
    /// container moves its contents (their bounds are relative). Locked
    /// objects do not move.
    fn movable(&self) -> BTreeSet<u64> {
        let l = &self.layout;
        l.objects
            .iter()
            .filter(|o| self.sel.contains(&o.id) && !Self::locked(o))
            .filter(|o| !ancestors(l, o).iter().any(|a| self.sel.contains(a)))
            .map(|o| o.id)
            .collect()
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
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
            shift(&mut o.bounds, dx, dy);
        }
    }

    /// Keyboard: arrows move (Shift = 10 pt), Delete, Escape, undo/redo,
    /// clipboard, duplicate.
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
        let (redo, undo, all, dup) = ctx.input_mut(|i| {
            let r = i.consume_key(cmd | Modifiers::SHIFT, Key::Z) || i.consume_key(cmd, Key::Y);
            (
                r,
                i.consume_key(cmd, Key::Z),
                i.consume_key(cmd, Key::A),
                i.consume_key(cmd, Key::D),
            )
        });
        // Copy, cut and paste arrive as events (egui turns the shortcuts into them).
        let (copy, cut, paste) = ctx.input(|i| {
            let mut c = (false, false, false);
            for e in &i.events {
                match e {
                    egui::Event::Copy => c.0 = true,
                    egui::Event::Cut => c.1 = true,
                    egui::Event::Paste(_) => c.2 = true,
                    egui::Event::Key {
                        key: Key::V,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command => c.2 = true,
                    _ => {}
                }
            }
            c
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if all {
            self.select_all();
        }
        if copy || cut {
            self.copy();
            ctx.copy_text(tr("BaseCraft layout objects").to_string());
        }
        if cut {
            self.delete_selection();
        }
        if paste {
            self.paste();
        }
        if dup {
            self.duplicate();
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
                for m in [Modifiers::NONE, Modifiers::SHIFT] {
                    while i.consume_key(m, k) {
                        a.0 += d.0;
                        a.1 += d.1;
                    }
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
            self.sel_part = None;
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

    pub fn set_tool(&mut self, t: Tool) {
        self.end_text_edit();
        self.tool = t;
    }

    /// Tool palette and view toggles shown in the status toolbar.
    pub fn tools(&mut self, ui: &mut egui::Ui) {
        for (t, icon, tip) in [
            (Tool::Select, "↖", "Selection tool"),
            (Tool::Text, "T", "Text tool"),
            (Tool::Line, "╱", "Line tool"),
            (Tool::Rect, "▭", "Rectangle tool"),
            (Tool::Oval, "( )", "Oval tool"),
            (Tool::Field, "[F]", "Field tool"),
            (Tool::Button, "(B)", "Button tool"),
            (Tool::Portal, "[P]", "Portal tool"),
            (Tool::TabControl, "[Tab]", "Tab control tool"),
            (Tool::PopoverButton, "[▾]", "Popover button tool"),
            (Tool::SlideControl, "[•••]", "Slide control tool"),
            (Tool::ButtonBar, "[|||]", "Button bar tool"),
            (Tool::WebViewer, "[www]", "Web viewer tool"),
        ] {
            if ui
                .selectable_label(self.tool == t, icon)
                .on_hover_text(tr(tip))
                .clicked()
            {
                self.set_tool(t);
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
        ui.separator();
        ui.checkbox(&mut self.show_rulers, tr("Rulers"));
        ui.checkbox(&mut self.show_grid, tr("Grid"));
        ui.checkbox(&mut self.snap_grid, tr("Snap to grid"));
    }

    pub fn open_setup(&mut self) {
        self.setup = Some(crate::layout_setup::Setup::new(&self.layout));
    }

    /// Layouts › New Layout/Report…: asks for a name and an occurrence.
    pub fn open_new_layout(&mut self, doc: &Doc) {
        let to = (self.layout.table_occurrence & 0xFFF) as u32;
        let base = doc
            .db
            .occurrences
            .iter()
            .find(|o| o.id == to)
            .map_or(tr("Layout").to_string(), |o| o.name.clone());
        let name = (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| doc.db.layouts.iter().all(|l| &l.name != n))
            .unwrap_or_default();
        self.dialog = Some(Dialog::NewLayout { name, to });
    }

    /// Layouts › Delete Layout…: asks first.
    pub fn open_delete_layout(&mut self) {
        self.dialog = Some(Dialog::DeleteLayout);
    }

    /// Insert › Part…
    pub fn open_part_setup(&mut self) {
        let have: Vec<u64> = self.layout.parts.iter().map(|p| p.kind).collect();
        let kind = PART_KINDS
            .iter()
            .copied()
            .find(|k| !have.contains(k))
            .unwrap_or(part::LEADING_SUB_SUMMARY);
        self.dialog = Some(Dialog::Part {
            kind,
            to: (self.layout.table_occurrence & 0xFFF) as u32,
            field: None,
        });
    }

    fn alloc_id(&mut self) -> u64 {
        let max = self.layout.objects.iter().map(|o| o.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
        self.next_id += 1;
        self.next_id - 1
    }

    /// Adds a new object on top of the others and selects it. New objects
    /// are anchored left and top, FileMaker's default.
    fn add(&mut self, mut o: LayoutObject) -> u64 {
        o.id = self.alloc_id();
        if o.options == 0 {
            o.options = anchor::LEFT | anchor::TOP;
        }
        let id = o.id;
        self.layout.objects.insert(0, o);
        self.sel = BTreeSet::from([id]);
        id
    }

    fn object(&self, id: u64) -> Option<&LayoutObject> {
        self.layout.objects.iter().find(|o| o.id == id)
    }

    fn object_mut(&mut self, id: u64) -> Option<&mut LayoutObject> {
        self.layout.objects.iter_mut().find(|o| o.id == id)
    }

    /// Topmost visible object under `p` (layout coordinates), preferring
    /// the innermost one (a field in a portal rather than the portal); a
    /// click inside a group picks the group.
    fn hit(&self, p: Pos2) -> Option<u64> {
        let l = &self.layout;
        let id = l
            .objects
            .iter()
            .filter(|o| o.kind != kind::TAB_PANEL && o.kind != kind::GROUP && visible(l, o))
            .filter(|o| rect_of(&l.absolute(o)).expand(3.0).contains(p))
            .max_by_key(|o| {
                let depth = ancestors(l, o).len();
                // Earlier in the list = more in front; max_by_key keeps the last max.
                let pos = l.objects.iter().position(|x| x.id == o.id).unwrap_or(0);
                (depth, usize::MAX - pos)
            })
            .map(|o| o.id)?;
        Some(self.outer_group(id))
    }

    /// The outermost group containing `id`, or `id`.
    fn outer_group(&self, id: u64) -> u64 {
        let l = &self.layout;
        let Some(o) = self.object(id) else { return id };
        ancestors(l, o)
            .into_iter()
            .rfind(|a| self.object(*a).is_some_and(|x| x.kind == kind::GROUP))
            .unwrap_or(id)
    }

    fn handle_at(&self, p: Pos2) -> Option<(u64, usize)> {
        for id in &self.sel {
            let Some(o) = self.object(*id) else { continue };
            if Self::locked(o) {
                continue;
            }
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

    /// Part that contains `y`.
    fn part_at(&self, y: f32) -> Option<usize> {
        self.layout
            .parts
            .iter()
            .position(|p| (p.top as f32..(p.top + p.height) as f32).contains(&y))
    }

    fn guide_at(&self, p: Pos2) -> Option<usize> {
        self.guides.iter().position(|g| {
            let v = if g.vertical { p.x } else { p.y };
            (v as f64 - g.at).abs() <= 3.0
        })
    }

    /// A press on the canvas. `ruler` is `Some(vertical)` when it starts
    /// on a ruler (a new guide).
    fn press(&mut self, p: Pos2, shift: bool, in_gutter: bool, ruler: Option<bool>) {
        self.end_text_edit();
        self.touched = None;
        if let Some(vertical) = ruler {
            let at = if vertical { p.x } else { p.y } as f64;
            self.guides.push(Guide { vertical, at });
            self.drag = Some(Drag::Guide(self.guides.len() - 1));
            return;
        }
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
        if in_gutter {
            // A click on a part label selects the part.
            self.sel.clear();
            self.sel_part = self.part_at(p.y);
            return;
        }
        if let Some(g) = self.guide_at(p).filter(|_| self.hit(p).is_none()) {
            self.drag = Some(Drag::Guide(g));
            return;
        }
        self.sel_part = None;
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

    /// Vertical (x) and horizontal (y) snap lines: the guides.
    fn snap_lines(&self) -> (Vec<f64>, Vec<f64>) {
        let xs = self
            .guides
            .iter()
            .filter(|g| g.vertical)
            .map(|g| g.at)
            .collect();
        let ys = self
            .guides
            .iter()
            .filter(|g| !g.vertical)
            .map(|g| g.at)
            .collect();
        (xs, ys)
    }

    /// The move (dx, dy) adjusted so that the moved box snaps: its
    /// left/top corner to the grid, its edges to the guides.
    fn snapped(&self, b: &Bounds, dx: f64, dy: f64) -> (f64, f64) {
        let (mut dx, mut dy) = (dx, dy);
        let g = GRID as f64;
        if self.snap_grid {
            dx = ((b.left + dx) / g).round() * g - b.left;
            dy = ((b.top + dy) / g).round() * g - b.top;
        }
        let (xs, ys) = self.snap_lines();
        let sx = [b.left + dx, b.right + dx]
            .iter()
            .filter_map(|v| snap_to(*v, &xs))
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        let sy = [b.top + dy, b.bottom + dy]
            .iter()
            .filter_map(|v| snap_to(*v, &ys))
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        (dx + sx.unwrap_or(0.0), dy + sy.unwrap_or(0.0))
    }

    /// A point snapped to the grid and the guides (creation, resizing).
    fn snap_point(&self, x: f64, y: f64) -> (f64, f64) {
        let g = GRID as f64;
        let (mut x, mut y) = (x, y);
        if self.snap_grid {
            x = (x / g).round() * g;
            y = (y / g).round() * g;
        }
        let (xs, ys) = self.snap_lines();
        (
            x + snap_to(x, &xs).unwrap_or(0.0),
            y + snap_to(y, &ys).unwrap_or(0.0),
        )
    }

    fn drag_to(&mut self, p: Pos2) {
        let movable = self.movable();
        match &self.drag {
            Some(Drag::Move { start, orig }) => {
                let d = (p - *start).round();
                let mut l = orig.clone();
                // Snap the selection's bounding box.
                let bb = union(
                    l.objects
                        .iter()
                        .filter(|o| movable.contains(&o.id))
                        .map(|o| l.absolute(o)),
                );
                let (dx, dy) = match bb {
                    Some(b) => self.snapped(&b, d.x as f64, d.y as f64),
                    None => (d.x as f64, d.y as f64),
                };
                for o in l.objects.iter_mut().filter(|o| movable.contains(&o.id)) {
                    shift(&mut o.bounds, dx, dy);
                }
                self.layout = l;
            }
            Some(Drag::Resize {
                id,
                handle,
                start,
                orig,
            }) => {
                let (id, handle) = (*id, *handle);
                let d = (p - *start).round();
                let mut l = orig.clone();
                let Some(o) = l.objects.iter().find(|o| o.id == id) else {
                    return;
                };
                let abs = l.absolute(o);
                let (ox, oy) = (abs.left - o.bounds.left, abs.top - o.bounds.top);
                let mut b = abs;
                let (x, y) = self.snap_point(
                    if matches!(handle, 0 | 6 | 7) {
                        b.left
                    } else {
                        b.right
                    } + d.x as f64,
                    if matches!(handle, 0..=2) {
                        b.top
                    } else {
                        b.bottom
                    } + d.y as f64,
                );
                if matches!(handle, 0 | 6 | 7) {
                    b.left = x.min(b.right - 1.0);
                }
                if matches!(handle, 2..=4) {
                    b.right = x.max(b.left + 1.0);
                }
                if matches!(handle, 0..=2) {
                    b.top = y.min(b.bottom);
                }
                if matches!(handle, 4..=6) {
                    b.bottom = y.max(b.top);
                }
                if let Some(o) = l.objects.iter_mut().find(|o| o.id == id) {
                    shift(&mut b, -ox, -oy);
                    o.bounds = b;
                }
                self.layout = l;
            }
            Some(Drag::Part { index, start, orig }) => {
                let p0 = &orig.parts[*index];
                let dy = ((p.y - start.y).round() as f64).max(1.0 - p0.height);
                let mut l = orig.clone();
                resize_part(&mut l, *index, dy);
                self.layout = l;
            }
            Some(Drag::Guide(i)) => {
                let i = *i;
                if let Some(g) = self.guides.get_mut(i) {
                    g.at = if g.vertical { p.x } else { p.y }.round() as f64;
                }
            }
            Some(Drag::Marquee { .. }) | Some(Drag::Create { .. }) => {
                if let Some(Drag::Marquee { now, .. } | Drag::Create { now, .. }) = &mut self.drag {
                    *now = p;
                }
            }
            None => {}
        }
    }

    fn release(&mut self) {
        match self.drag.take() {
            Some(Drag::Move { orig, .. }) => {
                if orig != self.layout {
                    self.reparent_moved();
                    self.undo.push(orig);
                    self.redo.clear();
                }
            }
            Some(Drag::Resize { orig, .. } | Drag::Part { orig, .. }) => {
                if orig != self.layout {
                    self.undo.push(orig);
                    self.redo.clear();
                }
            }
            Some(Drag::Guide(i)) => {
                // A guide dragged back onto its ruler goes away.
                if self.guides.get(i).is_some_and(|g| g.at < 0.0) {
                    self.guides.remove(i);
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
                    .filter(|o| o.kind != kind::TAB_PANEL && visible(l, o))
                    .filter(|o| m.contains_rect(rect_of(&l.absolute(o))))
                    .map(|o| self.outer_group(o.id))
                    .collect();
                if !add {
                    self.sel.clear();
                }
                self.sel.extend(inside);
            }
            Some(Drag::Create { start, now }) => {
                let (x0, y0) = self.snap_point(start.x as f64, start.y as f64);
                let (x1, y1) = self.snap_point(now.x as f64, now.y as f64);
                self.create(
                    egui::pos2(x0 as f32, y0 as f32),
                    egui::pos2(x1 as f32, y1 as f32),
                );
            }
            None => {}
        }
    }

    /// The innermost container (portal, front tab panel) that holds `b`
    /// (absolute), other than the objects in `not`.
    fn container_for(&self, b: &Bounds, not: &BTreeSet<u64>) -> Option<u64> {
        let l = &self.layout;
        let r = rect_of(b);
        l.objects
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    kind::PORTAL | kind::TAB_PANEL | kind::SLIDE_PANEL | 15
                )
            })
            .filter(|c| !not.contains(&c.id) && visible(l, c))
            .filter(|c| rect_of(&l.absolute(c)).expand(0.5).contains_rect(r))
            // The innermost, then the frontmost (first in the list).
            .max_by_key(|c| {
                let pos = l.objects.iter().position(|x| x.id == c.id).unwrap_or(0);
                (ancestors(l, c).len(), usize::MAX - pos)
            })
            .map(|c| c.id)
    }

    /// Puts `id` inside `parent` (or at the top level), keeping its place
    /// on the layout.
    fn set_parent(&mut self, id: u64, parent: Option<u64>) {
        let Some(o) = self.object(id) else { return };
        if o.parent == parent {
            return;
        }
        let abs = self.layout.absolute(o);
        let origin = parent
            .and_then(|p| self.object(p))
            .map(|p| self.layout.absolute(p))
            .map_or((0.0, 0.0), |b| (b.left, b.top));
        if let Some(o) = self.object_mut(id) {
            o.bounds = abs;
            shift(&mut o.bounds, -origin.0, -origin.1);
            o.parent = parent;
        }
    }

    /// Objects dropped into a portal or tab panel go inside it; dragged out,
    /// they come back to the top level. Group members stay in their group.
    fn reparent_moved(&mut self) {
        for id in self.movable() {
            let Some(o) = self.object(id) else { continue };
            if o.parent
                .and_then(|p| self.object(p))
                .is_some_and(|p| p.kind == kind::GROUP)
            {
                continue;
            }
            let b = self.layout.absolute(o);
            let not = subtree(&self.layout, id);
            let mut parent = self.container_for(&b, &not);
            // No portals or tab controls inside a portal.
            if matches!(o.kind, kind::PORTAL | kind::TAB_CONTROL)
                && parent
                    .and_then(|p| self.object(p))
                    .is_some_and(|p| p.kind == kind::PORTAL)
            {
                parent = None;
            }
            self.set_parent(id, parent);
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
            Tool::Field => (200.0, 44.0),
            Tool::Button => (100.0, 30.0),
            Tool::Line => (100.0, 0.0),
            Tool::Portal => (320.0, 150.0),
            Tool::TabControl => (360.0, 220.0),
            Tool::PopoverButton => (100.0, 30.0),
            Tool::SlideControl => (290.0, 170.0),
            Tool::ButtonBar => (300.0, 30.0),
            Tool::WebViewer => (300.0, 150.0),
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
        let layout_to = (self.layout.table_occurrence & 0xFFF) as u32;
        match tool {
            Tool::Text => {
                self.checkpoint();
                let text = tr("Text").to_string();
                let id = self.add(LayoutObject {
                    kind: kind::TEXT,
                    style_raw: new_style(70),
                    text: Some(text.clone()),
                    ..base
                });
                self.place_inside(id);
                self.text_edit = Some((id, text));
            }
            Tool::Rect | Tool::Oval | Tool::Line => {
                self.checkpoint();
                let (k, ty) = match tool {
                    Tool::Rect => (kind::RECT, 51),
                    Tool::Oval => (kind::OVAL, 53),
                    _ => (kind::LINE, 50),
                };
                let id = self.add(LayoutObject {
                    kind: k,
                    style_raw: new_style(ty),
                    ..base
                });
                self.place_inside(id);
            }
            Tool::Field => {
                self.dialog = Some(Dialog::Field {
                    rect,
                    to: layout_to,
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
            Tool::Portal => {
                self.dialog = Some(Dialog::Portal {
                    rect,
                    to: 0,
                    rows: 3,
                    fields: Vec::new(),
                })
            }
            Tool::TabControl => {
                self.dialog = Some(Dialog::TabControl {
                    rect,
                    tabs: format!("{}\n{}", tr("Tab 1"), tr("Tab 2")),
                })
            }
            Tool::PopoverButton => self.insert_popover_button(rect),
            Tool::SlideControl => self.insert_slide_control(rect),
            Tool::ButtonBar => self.insert_button_bar(rect),
            Tool::WebViewer => self.new_web_setup(rect),
            Tool::Select => {}
        }
    }

    /// A new object drawn inside a portal or tab panel goes into it.
    fn place_inside(&mut self, id: u64) {
        let Some(o) = self.object(id) else { return };
        let b = self.layout.absolute(o);
        // Not into itself or its own panels.
        let parent = self.container_for(&b, &subtree(&self.layout, id));
        self.set_parent(id, parent);
    }

    // ---- Arrange -------------------------------------------------------

    /// Absolute bounds of the movable selection.
    fn selected_boxes(&self) -> Vec<(u64, Bounds)> {
        let m = self.movable();
        self.layout
            .objects
            .iter()
            .filter(|o| m.contains(&o.id))
            .map(|o| (o.id, self.layout.absolute(o)))
            .collect()
    }

    fn move_by(&mut self, id: u64, dx: f64, dy: f64) {
        if let Some(o) = self.object_mut(id) {
            shift(&mut o.bounds, dx, dy);
        }
    }

    /// Arrange › Align: to the selection's extreme edge or centre.
    pub fn align(&mut self, a: Align) {
        let boxes = self.selected_boxes();
        if boxes.len() < 2 {
            return;
        }
        let Some(bb) = union(boxes.iter().map(|b| b.1.clone())) else {
            return;
        };
        self.checkpoint();
        for (id, b) in boxes {
            let (dx, dy) = match a {
                Align::Left => (bb.left - b.left, 0.0),
                Align::Right => (bb.right - b.right, 0.0),
                Align::HCenter => ((bb.left + bb.right - b.left - b.right) / 2.0, 0.0),
                Align::Top => (0.0, bb.top - b.top),
                Align::Bottom => (0.0, bb.bottom - b.bottom),
                Align::VCenter => (0.0, (bb.top + bb.bottom - b.top - b.bottom) / 2.0),
            };
            self.move_by(id, dx.round(), dy.round());
        }
    }

    /// Arrange › Distribute: equal space between three or more objects,
    /// the first and last staying.
    pub fn distribute(&mut self, horizontal: bool) {
        let mut boxes = self.selected_boxes();
        if boxes.len() < 3 {
            return;
        }
        let key = |b: &Bounds| if horizontal { b.left } else { b.top };
        let size = |b: &Bounds| {
            if horizontal {
                b.right - b.left
            } else {
                b.bottom - b.top
            }
        };
        boxes.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
        let (first, last) = (&boxes[0].1, &boxes[boxes.len() - 1].1);
        let span = key(last) + size(last) - key(first);
        let used: f64 = boxes.iter().map(|b| size(&b.1)).sum();
        let gap = (span - used) / (boxes.len() - 1) as f64;
        self.checkpoint();
        let mut at = key(first);
        for (id, b) in boxes {
            let d = (at - key(&b)).round();
            if horizontal {
                self.move_by(id, d, 0.0);
            } else {
                self.move_by(id, 0.0, d);
            }
            at += size(&b) + gap;
        }
    }

    /// Arrange › Resize To: the smallest or largest width and/or height
    /// of the selection.
    pub fn resize_to(&mut self, largest: bool, width: bool, height: bool) {
        let boxes = self.selected_boxes();
        if boxes.len() < 2 {
            return;
        }
        let pick = |v: Vec<f64>| -> f64 {
            v.into_iter()
                .reduce(|a, b| if largest { a.max(b) } else { a.min(b) })
                .unwrap_or(0.0)
        };
        let w = pick(boxes.iter().map(|b| b.1.right - b.1.left).collect());
        let h = pick(boxes.iter().map(|b| b.1.bottom - b.1.top).collect());
        self.checkpoint();
        for (id, _) in boxes {
            if let Some(o) = self.object_mut(id) {
                if width {
                    o.bounds.right = o.bounds.left + w;
                }
                if height {
                    o.bounds.bottom = o.bounds.top + h;
                }
            }
        }
    }

    /// Arrange › Bring to Front / Send to Back / Forward / Backward. The
    /// object list is front to back; an object keeps its contents just in
    /// front of it.
    pub fn arrange_z(&mut self, to_front: bool, all_the_way: bool) {
        let ids = self.movable();
        if ids.is_empty() {
            return;
        }
        self.checkpoint();
        for id in ids {
            let block = subtree(&self.layout, id);
            let pos = |objs: &[LayoutObject]| objs.iter().position(|o| block.contains(&o.id));
            let (moved, rest): (Vec<LayoutObject>, Vec<LayoutObject>) = self
                .layout
                .objects
                .iter()
                .cloned()
                .partition(|o| block.contains(&o.id));
            let at = pos(&self.layout.objects).unwrap_or(0);
            let at = match (to_front, all_the_way) {
                (true, true) => 0,
                (false, true) => rest.len(),
                (true, false) => at.saturating_sub(1),
                (false, false) => (at + 1).min(rest.len()),
            };
            let mut objs = rest;
            for (i, o) in moved.into_iter().enumerate() {
                objs.insert(at + i, o);
            }
            self.layout.objects = objs;
        }
    }

    /// Arrange › Group: the selected objects (two or more, in the same
    /// container) become one group object (kind 8) holding them.
    pub fn group(&mut self) {
        let ids = self.movable();
        if ids.len() < 2 {
            return;
        }
        let parents: BTreeSet<Option<u64>> = ids
            .iter()
            .filter_map(|i| self.object(*i))
            .map(|o| o.parent)
            .collect();
        let [parent] = parents.into_iter().collect::<Vec<_>>()[..] else {
            return;
        };
        let Some(bb) = union(
            ids.iter()
                .filter_map(|i| self.object(*i))
                .map(|o| o.bounds.clone()),
        ) else {
            return;
        };
        self.checkpoint();
        let front = self
            .layout
            .objects
            .iter()
            .position(|o| ids.contains(&o.id))
            .unwrap_or(0);
        let gid = self.alloc_id();
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| ids.contains(&o.id))
        {
            shift(&mut o.bounds, -bb.left, -bb.top);
            o.parent = Some(gid);
        }
        self.layout.objects.insert(
            front,
            LayoutObject {
                id: gid,
                kind: kind::GROUP,
                options: anchor::LEFT | anchor::TOP,
                bounds: bb,
                parent,
                ..Default::default()
            },
        );
        self.sel = BTreeSet::from([gid]);
    }

    /// Arrange › Ungroup.
    pub fn ungroup(&mut self) {
        let groups: Vec<u64> = self
            .sel
            .iter()
            .copied()
            .filter(|i| self.object(*i).is_some_and(|o| o.kind == kind::GROUP))
            .collect();
        if groups.is_empty() {
            return;
        }
        self.checkpoint();
        for g in groups {
            let Some(go) = self.object(g).cloned() else {
                continue;
            };
            let mut members = BTreeSet::new();
            for o in self
                .layout
                .objects
                .iter_mut()
                .filter(|o| o.parent == Some(g))
            {
                shift(&mut o.bounds, go.bounds.left, go.bounds.top);
                o.parent = go.parent;
                members.insert(o.id);
            }
            self.layout.objects.retain(|o| o.id != g);
            self.sel.remove(&g);
            self.sel.extend(members);
        }
    }

    /// Arrange › Lock / Unlock.
    pub fn set_locked(&mut self, lock: bool) {
        let ids = self.sel.clone();
        if ids.is_empty() {
            return;
        }
        self.checkpoint();
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| ids.contains(&o.id))
        {
            if lock {
                o.options |= fmp12::layouts::LOCKED;
            } else {
                o.options &= !fmp12::layouts::LOCKED;
            }
        }
    }

    // ---- Clipboard -------------------------------------------------------

    /// Edit › Copy: the selected objects with their contents.
    pub fn copy(&self) {
        let l = &self.layout;
        let top = self.movable_or_locked();
        let ids: BTreeSet<u64> = top.iter().flat_map(|id| subtree(l, *id)).collect();
        let objs: Vec<LayoutObject> = l
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .map(|o| {
                let mut o = o.clone();
                // Copied objects keep their place on the layout.
                if top.contains(&o.id) {
                    o.bounds = l.absolute(&o);
                    o.parent = None;
                }
                o
            })
            .collect();
        if !objs.is_empty()
            && let Ok(mut c) = CLIPBOARD.lock()
        {
            *c = objs;
        }
    }

    fn movable_or_locked(&self) -> BTreeSet<u64> {
        let l = &self.layout;
        l.objects
            .iter()
            .filter(|o| self.sel.contains(&o.id))
            .filter(|o| !ancestors(l, o).iter().any(|a| self.sel.contains(a)))
            .map(|o| o.id)
            .collect()
    }

    /// Edit › Paste: the copied objects with new ids, selected. Pasted onto
    /// the layout they came from, they land 10 points down and right of
    /// any object still at their place.
    pub fn paste(&mut self) {
        let objs = CLIPBOARD.lock().map(|c| c.clone()).unwrap_or_default();
        self.paste_objects(objs, 0.0);
    }

    fn paste_objects(&mut self, objs: Vec<LayoutObject>, offset: f64) {
        if objs.is_empty() {
            return;
        }
        self.checkpoint();
        let occupied = objs.iter().filter(|o| o.parent.is_none()).any(|o| {
            self.layout
                .objects
                .iter()
                .any(|x| x.parent.is_none() && x.bounds == o.bounds)
        });
        let d = if offset > 0.0 || occupied { 10.0 } else { 0.0 };
        let mut map = BTreeMap::new();
        for o in &objs {
            let id = self.alloc_id();
            map.insert(o.id, id);
        }
        let mut new: Vec<LayoutObject> = objs
            .into_iter()
            .map(|mut o| {
                o.id = map[&o.id];
                o.parent = o.parent.and_then(|p| map.get(&p).copied());
                o.active_child = o.active_child.and_then(|p| map.get(&p).copied());
                o.options &= !fmp12::layouts::LOCKED;
                if o.parent.is_none() {
                    shift(&mut o.bounds, d, d);
                }
                o
            })
            .collect();
        self.sel = new
            .iter()
            .filter(|o| o.parent.is_none())
            .map(|o| o.id)
            .collect();
        new.append(&mut self.layout.objects);
        self.layout.objects = new;
        self.sel_part = None;
    }

    /// Edit › Duplicate (Ctrl+D): a copy 10 points down and right.
    pub fn duplicate(&mut self) {
        let before = CLIPBOARD.lock().map(|c| c.clone()).unwrap_or_default();
        self.copy();
        let objs = CLIPBOARD.lock().map(|c| c.clone()).unwrap_or_default();
        if let Ok(mut c) = CLIPBOARD.lock() {
            *c = before;
        }
        if self.sel.is_empty() {
            return;
        }
        self.paste_objects(objs, 10.0);
    }

    // ---- Parts -------------------------------------------------------

    /// Adds a part of `kind` (with its break field for sub-summaries) at
    /// its place in FileMaker's part order, 40 points high; the parts and
    /// objects below move down.
    pub fn add_part(&mut self, kind: u64, break_field: Option<(u32, u32)>) {
        let rank = fmp12::layouts::part_rank(kind);
        let unique = !matches!(kind, part::LEADING_SUB_SUMMARY | part::TRAILING_SUB_SUMMARY);
        if unique && self.layout.parts.iter().any(|p| p.kind == kind) {
            return;
        }
        let i = self
            .layout
            .parts
            .iter()
            .position(|p| fmp12::layouts::part_rank(p.kind) > rank)
            .unwrap_or(self.layout.parts.len());
        let top = if i == 0 {
            0.0
        } else {
            let p = &self.layout.parts[i - 1];
            p.top + p.height
        };
        self.checkpoint();
        let h = 40.0;
        for p in self.layout.parts.iter_mut().skip(i) {
            p.top += h;
        }
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| o.parent.is_none() && o.bounds.top >= top - 0.5)
        {
            shift(&mut o.bounds, 0.0, h);
        }
        self.layout.parts.insert(
            i,
            Part {
                kind,
                top,
                height: h,
                break_field,
                ..Default::default()
            },
        );
        self.sel.clear();
        self.sel_part = Some(i);
    }

    /// Deletes part `i` and the objects that start in it; the parts and
    /// objects below move up. The last part stays.
    pub fn delete_part(&mut self, i: usize) {
        if self.layout.parts.len() <= 1 || i >= self.layout.parts.len() {
            return;
        }
        self.checkpoint();
        let p = self.layout.parts.remove(i);
        let (top, bottom) = (p.top, p.top + p.height);
        let l = &self.layout;
        let gone: BTreeSet<u64> = l
            .objects
            .iter()
            .filter(|o| o.parent.is_none() && o.bounds.top >= top && o.bounds.top < bottom)
            .flat_map(|o| subtree(l, o.id))
            .collect();
        self.layout.objects.retain(|o| !gone.contains(&o.id));
        for q in self.layout.parts.iter_mut().skip(i) {
            q.top -= p.height;
        }
        for o in self
            .layout
            .objects
            .iter_mut()
            .filter(|o| o.parent.is_none() && o.bounds.top >= bottom - 0.5)
        {
            shift(&mut o.bounds, 0.0, -p.height);
        }
        self.sel_part = None;
        self.sel.retain(|x| !gone.contains(x));
    }

    /// The layout canvas.
    pub fn canvas(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        let l = self.layout.clone();
        let total = l.parts.iter().map(|p| p.top + p.height).fold(0.0, f64::max) as f32;
        let ruler = if self.show_rulers { RULER } else { 0.0 };
        let size = Vec2::new(ruler + GUTTER + l.width as f32 + 60.0, ruler + total + 60.0);
        let (resp, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
        let o0 = resp.rect.min + Vec2::new(ruler + GUTTER, ruler);
        self.canvas_origin = o0;
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
        if self.show_grid {
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
        }
        // Objects, back to front.
        let layout_to = (l.table_occurrence & 0xFFF) as u32;
        let table = doc.db.table_of(layout_to).map(|t| t.id).unwrap_or(0);
        for o in l.objects.iter().rev() {
            if !visible(&l, o) || matches!(o.kind, kind::TAB_PANEL | kind::GROUP) {
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
            let active = matches!(&self.drag, Some(Drag::Part { index, .. }) if *index == i)
                || self.sel_part == Some(i);
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
        // Guides.
        let guide = Stroke::new(1.0, Color32::from_rgb(230, 60, 160));
        for g in &self.guides {
            if g.vertical {
                painter.vline(o0.x + g.at as f32, resp.rect.y_range(), guide);
            } else {
                painter.hline(resp.rect.x_range(), o0.y + g.at as f32, guide);
            }
        }
        // Rulers (points).
        if self.show_rulers {
            self.rulers(&painter, resp.rect, o0);
        }
        // Selection handles; locked objects get grey ones.
        let blue = Color32::from_rgb(30, 110, 230);
        for id in &self.sel {
            let Some(o) = l.objects.iter().find(|o| o.id == *id) else {
                continue;
            };
            let r = to_screen(rect_of(&l.absolute(o)));
            let c = if Self::locked(o) {
                Color32::from_gray(140)
            } else {
                blue
            };
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, c), StrokeKind::Outside);
            for h in handles(r) {
                let hr = Rect::from_center_size(h, Vec2::splat(HANDLE));
                painter.rect_filled(hr, 0.0, Color32::WHITE);
                painter.rect_stroke(hr, 0.0, Stroke::new(1.0, c), StrokeKind::Inside);
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
        let ruler_of = |p: Pos2| -> Option<bool> {
            if !self.show_rulers {
                return None;
            }
            if p.y < resp.rect.min.y + RULER && p.x >= o0.x {
                Some(false)
            } else if p.x < resp.rect.min.x + RULER && p.y >= o0.y {
                Some(true)
            } else {
                None
            }
        };
        if resp.drag_started() || resp.clicked() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos());
            if let Some(p) = origin {
                let r = if resp.drag_started() {
                    ruler_of(p)
                } else {
                    None
                };
                self.press(to_layout(p), shift, p.x < o0.x && r.is_none(), r);
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
                kind::TEXT => self.text_edit = Some((id, o.text.clone().unwrap_or_default())),
                fmp12::layouts::kind::EXTERNAL
                    if o.external
                        .is_some_and(|e| e.0 == fmp12::layouts::WEB_VIEWER) =>
                {
                    let b = o.bounds.clone();
                    self.open_web_setup(b, Some(id), doc);
                }
                kind::BUTTON => {
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

    fn rulers(&self, painter: &egui::Painter, area: Rect, o0: Pos2) {
        let ruler_bg = Color32::from_gray(245);
        let top_ruler = Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + RULER));
        let left_ruler = Rect::from_min_max(area.min, egui::pos2(area.min.x + RULER, area.max.y));
        painter.rect_filled(top_ruler, 0.0, ruler_bg);
        painter.rect_filled(left_ruler, 0.0, ruler_bg);
        let tick = Stroke::new(0.5, Color32::from_gray(110));
        let small = egui::FontId::proportional(9.0);
        let len = |v: f32| {
            if v as i32 % 100 == 0 {
                RULER
            } else if v as i32 % 50 == 0 {
                8.0
            } else {
                4.0
            }
        };
        let mut v = 0.0;
        while o0.x + v <= area.max.x {
            painter.vline(
                o0.x + v,
                (top_ruler.bottom() - len(v))..=top_ruler.bottom(),
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
        while o0.y + v <= area.max.y {
            painter.hline(
                (left_ruler.right() - len(v))..=left_ruler.right(),
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
    }

    // ---- Menus ---------------------------------------------------------

    /// Insert menu (Layout mode).
    pub fn insert_menu(&mut self, ui: &mut egui::Ui) {
        for (t, name) in [
            (Tool::Field, "Field…"),
            (Tool::Portal, "Portal…"),
            (Tool::TabControl, "Tab Control…"),
            (Tool::SlideControl, "Slide Control"),
            (Tool::WebViewer, "Web Viewer…"),
            (Tool::Button, "Button…"),
            (Tool::PopoverButton, "Popover Button"),
            (Tool::ButtonBar, "Button Bar"),
            (Tool::Text, "Text"),
            (Tool::Line, "Line"),
            (Tool::Rect, "Rectangle"),
            (Tool::Oval, "Oval"),
        ] {
            if ui.button(tr(name)).clicked() {
                self.set_tool(t);
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tr("Part…")).clicked() {
            self.open_part_setup();
            ui.close();
        }
    }

    /// Arrange menu (Layout mode).
    pub fn arrange_menu(&mut self, ui: &mut egui::Ui) {
        self.tab_order_item(ui);
        ui.separator();
        let n = self.movable().len();
        let item = |ui: &mut egui::Ui, on: bool, name: &'static str, keys: &str| {
            ui.add_enabled(on, egui::Button::new(tr(name)).shortcut_text(keys))
                .clicked()
        };
        if item(ui, n > 0, "Bring to Front", "Ctrl+Alt+Shift+[") {
            self.arrange_z(true, true);
            ui.close();
        }
        if item(ui, n > 0, "Bring Forward", "Ctrl+Shift+[") {
            self.arrange_z(true, false);
            ui.close();
        }
        if item(ui, n > 0, "Send Backward", "Ctrl+Shift+]") {
            self.arrange_z(false, false);
            ui.close();
        }
        if item(ui, n > 0, "Send to Back", "Ctrl+Alt+Shift+]") {
            self.arrange_z(false, true);
            ui.close();
        }
        ui.separator();
        ui.menu_button(tr("Align"), |ui| {
            for (a, name) in [
                (Align::Left, "Left Edges"),
                (Align::HCenter, "Centers (horizontally)"),
                (Align::Right, "Right Edges"),
                (Align::Top, "Top Edges"),
                (Align::VCenter, "Centers (vertically)"),
                (Align::Bottom, "Bottom Edges"),
            ] {
                if ui.add_enabled(n > 1, egui::Button::new(tr(name))).clicked() {
                    self.align(a);
                    ui.close();
                }
            }
        });
        ui.menu_button(tr("Distribute"), |ui| {
            if ui
                .add_enabled(n > 2, egui::Button::new(tr("Horizontally")))
                .clicked()
            {
                self.distribute(true);
                ui.close();
            }
            if ui
                .add_enabled(n > 2, egui::Button::new(tr("Vertically")))
                .clicked()
            {
                self.distribute(false);
                ui.close();
            }
        });
        ui.menu_button(tr("Resize To"), |ui| {
            for (largest, w, h, name) in [
                (false, true, false, "Smallest Width"),
                (false, false, true, "Smallest Height"),
                (false, true, true, "Smallest Width and Height"),
                (true, true, false, "Largest Width"),
                (true, false, true, "Largest Height"),
                (true, true, true, "Largest Width and Height"),
            ] {
                if ui.add_enabled(n > 1, egui::Button::new(tr(name))).clicked() {
                    self.resize_to(largest, w, h);
                    ui.close();
                }
            }
        });
        ui.separator();
        if item(ui, n > 1, "Group", "Ctrl+R") {
            self.group();
            ui.close();
        }
        let groups = self
            .sel
            .iter()
            .any(|i| self.object(*i).is_some_and(|o| o.kind == kind::GROUP));
        if item(ui, groups, "Ungroup", "Ctrl+Shift+R") {
            self.ungroup();
            ui.close();
        }
        let any = !self.sel.is_empty();
        if item(ui, any, "Lock", "Ctrl+Alt+L") {
            self.set_locked(true);
            ui.close();
        }
        if item(ui, any, "Unlock", "Ctrl+Alt+Shift+L") {
            self.set_locked(false);
            ui.close();
        }
    }

    /// Layouts menu: new, duplicate, delete, Layout Setup.
    pub fn layouts_menu(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        if ui.button(tr("New Layout/Report…")).clicked() {
            self.open_new_layout(doc);
            ui.close();
        }
        if ui.button(tr("Duplicate Layout")).clicked() {
            self.pending = Some(Then::Op(LayoutOp::Duplicate));
            ui.close();
        }
        if ui
            .add_enabled(
                doc.layouts.len() > 1,
                egui::Button::new(tr("Delete Layout…")),
            )
            .clicked()
        {
            self.open_delete_layout();
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Layout Setup…")).clicked() {
            self.open_setup();
            ui.close();
        }
    }

    /// The Inspector: Position, Styles, Appearance and Data tabs.
    pub fn inspector(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        ui.horizontal(|ui| {
            for (t, name) in [
                (Tab::Position, "Position"),
                (Tab::Styles, "Styles"),
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
            if let Some(i) = self.sel_part {
                self.part_inspector(ui, doc, i);
                return;
            }
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
            Tab::Position => {
                self.position_tab(ui, &ids);
                self.position_extra(ui, doc, &ids);
            }
            Tab::Styles => self.styles_tab(ui, doc, &ids),
            Tab::Appearance => self.appearance_tab(ui, doc, &ids),
            Tab::Data => self.data_tab(ui, doc, &ids),
        }
    }

    fn part_inspector(&mut self, ui: &mut egui::Ui, doc: &Doc, i: usize) {
        let Some(p) = self.layout.parts.get(i).cloned() else {
            return;
        };
        ui.label(format!("{}: {}", tr("Part"), part_name(p.kind)));
        ui.horizontal(|ui| {
            ui.label(tr("Height"));
            let mut h = p.height;
            if ui
                .add(
                    egui::DragValue::new(&mut h)
                        .range(1.0..=10000.0)
                        .suffix(" pt"),
                )
                .changed()
            {
                self.touch("part-height");
                resize_part(&mut self.layout, i, h - p.height);
            }
        });
        if let Some((to, f)) = p.break_field {
            let name = doc
                .db
                .table_of(to)
                .and_then(|t| t.field(f))
                .map_or("?".to_string(), |x| x.name.clone());
            ui.label(format!("{}: {name}", tr("Sorted by")));
        }
        if ui
            .add_enabled(
                self.layout.parts.len() > 1,
                egui::Button::new(tr("Delete Part")),
            )
            .clicked()
        {
            self.delete_part(i);
        }
    }

    fn position_tab(&mut self, ui: &mut egui::Ui, ids: &[u64]) {
        let id = ids[0];
        let Some(o) = self.object(id).cloned() else {
            return;
        };
        if ids.len() == 1 {
            ui.horizontal(|ui| {
                ui.label(tr("Name"));
                let mut n = o.name.clone();
                if ui.text_edit_singleline(&mut n).changed() {
                    self.touch("name");
                    if let Some(x) = self.object_mut(id) {
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
        // Shown in the units of Preferences › Layout (points inside).
        let (k, unit) = crate::prefs::unit();
        let mut shown = v.map(|x| x / k);
        let was = shown;
        egui::Grid::new("position").num_columns(4).show(ui, |ui| {
            for row in [[0usize, 1], [2, 3], [4, 5]] {
                for i in row {
                    ui.label(tr(["Left", "Top", "Right", "Bottom", "Width", "Height"][i]));
                    ui.add(
                        egui::DragValue::new(&mut shown[i])
                            .speed(1.0 / k)
                            .max_decimals(if k == 1.0 { 1 } else { 3 })
                            .suffix(unit),
                    );
                }
                ui.end_row();
            }
        });
        for i in 0..6 {
            if shown[i] != was[i] {
                v[i] = shown[i] * k;
            }
        }
        if v != before && !Self::locked(&o) {
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
            if let Some(x) = self.object_mut(id) {
                x.bounds = Bounds {
                    top: b.top - dy,
                    left: b.left - dx,
                    bottom: b.bottom - dy,
                    right: b.right - dx,
                };
            }
        }
        ui.separator();
        // Autosizing: the anchor bits of the object options.
        ui.label(tr("Autosizing"));
        ui.horizontal(|ui| {
            for (bit, name) in [
                (anchor::LEFT, "Left"),
                (anchor::TOP, "Top"),
                (anchor::RIGHT, "Right"),
                (anchor::BOTTOM, "Bottom"),
            ] {
                let mut on = o.options & bit != 0;
                if ui.checkbox(&mut on, tr(name)).changed() {
                    self.touch("anchors");
                    for x in self
                        .layout
                        .objects
                        .iter_mut()
                        .filter(|x| ids.contains(&x.id))
                    {
                        if on {
                            x.options |= bit;
                        } else {
                            x.options &= !bit;
                        }
                    }
                }
            }
        });
        let mut lock = Self::locked(&o);
        if ui.checkbox(&mut lock, tr("Lock")).changed() {
            self.set_locked(lock);
        }
        ui.separator();
        ui.label(tr("Sliding & visibility"));
        use fmp12::layouts::{HIDE_WHEN_PRINTING, SLIDE_LEFT, SLIDE_UP};
        for (bit, name) in [
            (SLIDE_LEFT, "Slide left"),
            (SLIDE_UP, "Slide up"),
            (HIDE_WHEN_PRINTING, "Hide when printing"),
        ] {
            let mut on = o.options & bit != 0;
            if ui.checkbox(&mut on, tr(name)).changed() {
                self.touch(name);
                for x in self
                    .layout
                    .objects
                    .iter_mut()
                    .filter(|x| ids.contains(&x.id))
                {
                    if on {
                        x.options |= bit;
                    } else {
                        x.options &= !bit;
                    }
                }
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

    /// Sets (or with `None` removes) a local `.self` property of the
    /// selected objects.
    fn set_prop(&mut self, ids: &[u64], what: &'static str, id: u32, v: Option<theme::Value>) {
        self.set_styles(ids, what, |s| {
            theme::with_local_prop(s, SELF, NORMAL, id, v.as_ref())
        });
    }

    fn styles_tab(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        let theme = doc.db.themes.iter().find(|t| t.number == self.layout.theme);
        if let Some(t) = theme {
            ui.label(format!("{}: {}", tr("Theme"), t.name));
        }
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
        let mut pick = None;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
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
        if !style.local.is_empty()
            && ui
                .button(tr("Clear local changes"))
                .on_hover_text(tr("Use the style's values"))
                .clicked()
        {
            self.set_styles(ids, "clear", |s| {
                let mut m = fmp12::pb::MsgBuf::parse(s).unwrap_or_default();
                m.set(1, fmp12::pb::Field::Bytes(LOCAL_613[2..].to_vec()));
                m.encode()
            });
        }
    }

    fn appearance_tab(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        let theme = doc.db.themes.iter().find(|t| t.number == self.layout.theme);
        let style = ObjectStyle::parse(&o.style_raw);
        let lk = look::resolve(theme, &style);
        let props = theme
            .map(|t| t.resolve(&style, SELF, NORMAL))
            .unwrap_or_default();
        let local = style
            .local
            .get(&(SELF, NORMAL))
            .cloned()
            .unwrap_or_default();
        let reset = |ui: &mut egui::Ui, has: bool| {
            ui.add_enabled(has, egui::Button::new("×"))
                .on_hover_text(tr("Use the style's value"))
                .clicked()
        };
        egui::Grid::new("appearance").num_columns(3).show(ui, |ui| {
            // Fill.
            ui.label(tr("Fill"));
            let mut c = lk.background.unwrap_or(Color32::TRANSPARENT);
            if ui.color_edit_button_srgba(&mut c).changed() {
                self.set_prop(
                    ids,
                    "fill",
                    prop::BACKGROUND_COLOR,
                    Some(theme::Value::Color(rgba(c))),
                );
            }
            if reset(ui, local.color(prop::BACKGROUND_COLOR).is_some()) {
                self.set_prop(ids, "fill-reset", prop::BACKGROUND_COLOR, None);
            }
            ui.end_row();
            // Line: style, colour and width on all four sides.
            ui.label(tr("Line"));
            let (w0, c0) = lk.borders[0];
            let st0 = props.enum_value(prop::BORDER_STYLE).unwrap_or(1);
            let mut st = if w0 > 0.0 && st0 != 1 { st0 } else { 1 };
            let mut c = c0;
            let mut w = w0 as f64;
            ui.horizontal(|ui| {
                let names = [
                    (1, "None"),
                    (5, "Solid"),
                    (4, "Dashed"),
                    (3, "Dotted"),
                    (6, "Double"),
                ];
                let label = names.iter().find(|n| n.0 == st).map_or("…", |n| n.1);
                let sr = egui::ComboBox::from_id_salt("line_style")
                    .selected_text(tr(label))
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        let mut ch = false;
                        for (v, n) in names {
                            ch |= ui.selectable_value(&mut st, v, tr(n)).changed();
                        }
                        ch
                    })
                    .inner
                    .unwrap_or(false);
                let cr = ui.color_edit_button_srgba(&mut c);
                let wr = ui.add(
                    egui::DragValue::new(&mut w)
                        .range(0.0..=20.0)
                        .speed(0.25)
                        .suffix(" pt"),
                );
                if sr || cr.changed() || wr.changed() {
                    if sr && st != 1 && w == 0.0 {
                        w = 1.0;
                    }
                    let (cv, wv, sv) =
                        (theme::Value::Color(rgba(c)), pt(w), theme::Value::Enum(st));
                    self.set_styles(ids, "border", |s| {
                        let mut s = s.to_vec();
                        for i in 0..4 {
                            for (p, v) in [
                                (prop::BORDER_COLOR, &cv),
                                (prop::BORDER_STYLE, &sv),
                                (prop::BORDER_WIDTH, &wv),
                            ] {
                                s = theme::with_local_prop(&s, SELF, NORMAL, p + i, Some(v));
                            }
                        }
                        s
                    });
                }
            });
            ui.label("");
            ui.end_row();
            // Corner radius, all four corners.
            ui.label(tr("Radius"));
            let mut r = lk.radius as f64;
            if ui
                .add(
                    egui::DragValue::new(&mut r)
                        .range(0.0..=200.0)
                        .suffix(" pt"),
                )
                .changed()
            {
                let l = Length {
                    value: r,
                    unit: Unit::Pt,
                };
                let v = theme::Value::Pair(l, l);
                self.set_styles(ids, "radius", |s| {
                    let mut s = s.to_vec();
                    for i in 0..4 {
                        s = theme::with_local_prop(
                            &s,
                            SELF,
                            NORMAL,
                            prop::BORDER_RADIUS + i,
                            Some(&v),
                        );
                    }
                    s
                });
            }
            ui.label("");
            ui.end_row();
        });
        ui.separator();
        ui.label(tr("Text"));
        egui::Grid::new("appearance_text")
            .num_columns(2)
            .show(ui, |ui| {
                // Font family: an entry of the file's font catalog.
                ui.label(tr("Font"));
                let cur = props.enum_value(prop::FONT_FAMILY);
                let name = |i: Option<i64>| {
                    i.and_then(|i| self.fonts.iter().find(|f| f.0 == i))
                        .map_or(tr("Default").to_string(), |f| {
                            f.1.split(',').next().unwrap_or_default().to_string()
                        })
                };
                let mut pick = None;
                egui::ComboBox::from_id_salt("font")
                    .selected_text(name(cur))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for (i, f) in &self.fonts {
                            let n = f.split(',').next().unwrap_or_default();
                            if ui.selectable_label(cur == Some(*i), n).clicked() {
                                pick = Some(*i);
                            }
                        }
                    });
                if let Some(i) = pick {
                    self.set_prop(ids, "font", prop::FONT_FAMILY, Some(theme::Value::Enum(i)));
                }
                ui.end_row();
                ui.label(tr("Size"));
                let mut sz = lk.font_size as f64;
                if ui
                    .add(
                        egui::DragValue::new(&mut sz)
                            .range(4.0..=500.0)
                            .suffix(" pt"),
                    )
                    .changed()
                {
                    self.set_prop(ids, "size", prop::FONT_SIZE, Some(pt(sz)));
                }
                ui.end_row();
                ui.label(tr("Style"));
                ui.horizontal(|ui| {
                    let bold = props.enum_value(prop::FONT_WEIGHT) == Some(2);
                    let italic = props.enum_value(prop::FONT_STYLE) == Some(2);
                    let under = props.enum_value(prop::UNDERLINE).unwrap_or(0) != 0;
                    let strike = props.enum_value(prop::STRIKETHROUGH).unwrap_or(0) != 0;
                    for (on, label, p, yes, no, what) in [
                        (bold, "B", prop::FONT_WEIGHT, 2, 1, "bold"),
                        (italic, "I", prop::FONT_STYLE, 2, 1, "italic"),
                        (under, "U", prop::UNDERLINE, 1, 0, "underline"),
                        (strike, "S", prop::STRIKETHROUGH, 1, 0, "strike"),
                    ] {
                        let tip = match what {
                            "bold" => "Bold",
                            "italic" => "Italic",
                            "underline" => "Underline",
                            _ => "Strikethrough",
                        };
                        if ui
                            .selectable_label(on, egui::RichText::new(label).strong())
                            .on_hover_text(tr(tip))
                            .clicked()
                        {
                            let v = if on { no } else { yes };
                            self.set_prop(ids, what, p, Some(theme::Value::Enum(v)));
                        }
                    }
                });
                ui.end_row();
                ui.label(tr("Color"));
                let mut c = lk.color;
                if ui.color_edit_button_srgba(&mut c).changed() {
                    let v = theme::Value::Color(rgba(c));
                    self.set_styles(ids, "text", |s| {
                        let s = theme::with_local_prop(s, TEXT, NORMAL, prop::COLOR, None);
                        theme::with_local_prop(&s, SELF, NORMAL, prop::COLOR, Some(&v))
                    });
                }
                ui.end_row();
                ui.label(tr("Align"));
                ui.horizontal(|ui| {
                    let cur = props.enum_value(prop::TEXT_ALIGN).unwrap_or(1);
                    for (v, label, tip) in [
                        (1, "L", "Align Left"),
                        (2, "C", "Center"),
                        (3, "R", "Align Right"),
                        (4, "J", "Justify"),
                    ] {
                        if ui
                            .selectable_label(cur == v, label)
                            .on_hover_text(tr(tip))
                            .clicked()
                        {
                            self.set_prop(
                                ids,
                                "align",
                                prop::TEXT_ALIGN,
                                Some(theme::Value::Enum(v)),
                            );
                        }
                    }
                });
                ui.end_row();
            });
    }

    fn data_tab(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        if o.kind == kind::PORTAL {
            self.portal_data(ui, doc, &o);
            return;
        }
        let Some((to_ref, field, rep)) = o.field else {
            ui.label(tr("No field selected"));
            self.data_extra(ui, doc, ids);
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
            if let Some(x) = self.object_mut(o.id) {
                x.field = Some((0x104000 | new_to as u64, new_field as u64, rep));
            }
        }
        ui.separator();
        ui.label(tr("Control style"));
        // The concealed edit box (last) is an edit box with a flag.
        let last = CONTROLS.len() - 1;
        let cur = if crate::controls::concealed(&o) {
            Some(last)
        } else {
            CONTROLS[..last].iter().position(|c| c.0 == o.display)
        };
        let mut d = cur;
        egui::ComboBox::from_id_salt("data_display")
            .selected_text(cur.map_or("…", |i| tr(CONTROLS[i].2)))
            .show_ui(ui, |ui| {
                for (i, c) in CONTROLS.iter().enumerate() {
                    ui.selectable_value(&mut d, Some(i), tr(c.2));
                }
            });
        if d != cur
            && let Some(i) = d
        {
            self.touch("display");
            let (display, ty, _) = CONTROLS[i];
            for x in self
                .layout
                .objects
                .iter_mut()
                .filter(|x| ids.contains(&x.id) && x.kind == kind::FIELD)
            {
                x.display = display;
                x.style_raw = theme::with_object_type(&x.style_raw, ty);
                x.field_flags &= !field_flags::CONCEALED;
                if i == last {
                    x.field_flags |= field_flags::CONCEALED;
                }
            }
        }
        if o.display != 0 && o.display != 6 {
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
                    .filter(|x| ids.contains(&x.id) && x.kind == kind::FIELD)
                {
                    x.value_list = vl;
                }
            }
        }
        ui.separator();
        self.data_extra(ui, doc, ids);
    }

    fn portal_data(&mut self, ui: &mut egui::Ui, doc: &Doc, o: &LayoutObject) {
        let Some((to, rows, first)) = o.portal else {
            return;
        };
        let t = (to & 0xFFF) as u32;
        ui.label(tr("Show related records from"));
        let mut nt = t;
        egui::ComboBox::from_id_salt("portal_to")
            .selected_text(
                doc.db
                    .occurrences
                    .iter()
                    .find(|x| x.id == t)
                    .map_or("?", |x| x.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for x in related(doc, (self.layout.table_occurrence & 0xFFF) as u32) {
                    ui.selectable_value(&mut nt, x.0, x.1);
                }
            });
        let (mut r, mut f) = (rows, first);
        ui.horizontal(|ui| {
            ui.label(tr("Initial row"));
            ui.add(egui::DragValue::new(&mut f).range(1..=10000));
            ui.label(tr("Number of rows"));
            ui.add(egui::DragValue::new(&mut r).range(1..=200));
        });
        if (nt, r, f) != (t, rows, first) {
            self.touch("portal");
            if let Some(x) = self.object_mut(o.id) {
                x.portal = Some((0x104000 | nt as u64, r, f));
            }
        }
    }

    /// Specify Field, Portal Setup, Tab Control Setup, Part Setup, Button
    /// Setup, New Layout, Delete Layout and Layout Setup dialogs.
    pub fn dialogs(&mut self, ctx: &egui::Context, doc: &Doc, scripts: &[(u32, String)]) {
        self.object_dialogs(ctx, doc);
        let mut close = false;
        let mut done = false;
        let layout_to = (self.layout.table_occurrence & 0xFFF) as u32;
        let occurrence = |id: u32| {
            doc.db
                .occurrences
                .iter()
                .find(|t| t.id == id)
                .map_or("?".to_string(), |t| t.name.clone())
        };
        let buttons = |ui: &mut egui::Ui, ok: bool, done: &mut bool, close: &mut bool| {
            ui.horizontal(|ui| {
                if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                    *done = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    *close = true;
                }
            });
        };
        let fields_of = |to: u32| -> Vec<(u32, String)> {
            doc.db
                .table_of(to)
                .map(|t| t.fields.iter().map(|f| (f.id, f.name.clone())).collect())
                .unwrap_or_default()
        };
        let occurrences = |ui: &mut egui::Ui, salt: &str, to: &mut u32| -> bool {
            let mut changed = false;
            egui::ComboBox::from_id_salt(salt)
                .selected_text(occurrence(*to))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for t in &doc.db.occurrences {
                        changed |= ui.selectable_value(to, t.id, &t.name).changed();
                    }
                });
            changed
        };
        match &mut self.dialog {
            Some(Dialog::Field {
                to, field, label, ..
            }) => {
                egui::Modal::new(egui::Id::NULL.with("specify_field")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Specify Field"));
                    if occurrences(ui, "sf_to", to) {
                        *field = None;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for (id, name) in fields_of(*to) {
                                let r = ui.selectable_label(*field == Some(id), name);
                                if r.clicked() {
                                    *field = Some(id);
                                }
                                if r.double_clicked() {
                                    done = true;
                                }
                            }
                        });
                    ui.checkbox(label, tr("Create label"));
                    buttons(ui, field.is_some(), &mut done, &mut close);
                });
            }
            Some(Dialog::Portal {
                to, rows, fields, ..
            }) => {
                egui::Modal::new(egui::Id::NULL.with("portal_setup")).show(ctx, |ui| {
                    ui.set_min_width(340.0);
                    ui.heading(tr("Portal Setup"));
                    ui.label(tr("Show related records from"));
                    egui::ComboBox::from_id_salt("ps_to")
                        .selected_text(if *to == 0 {
                            tr("<Select>").to_string()
                        } else {
                            occurrence(*to)
                        })
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for (id, name) in related(doc, layout_to) {
                                if ui.selectable_value(to, id, name).changed() {
                                    fields.clear();
                                }
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label(tr("Number of rows"));
                        ui.add(egui::DragValue::new(rows).range(1..=200));
                    });
                    ui.label(tr("Fields in the portal"));
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for (id, name) in fields_of(*to) {
                                let mut on = fields.contains(&id);
                                if ui.checkbox(&mut on, name).changed() {
                                    if on {
                                        fields.push(id);
                                    } else {
                                        fields.retain(|f| *f != id);
                                    }
                                }
                            }
                        });
                    buttons(ui, *to != 0, &mut done, &mut close);
                });
            }
            Some(Dialog::TabControl { tabs, .. }) => {
                egui::Modal::new(egui::Id::NULL.with("tab_setup")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Tab Control Setup"));
                    ui.label(tr("Tabs (one per line)"));
                    ui.add(egui::TextEdit::multiline(tabs).desired_rows(4));
                    let ok = tabs.lines().any(|l| !l.trim().is_empty());
                    buttons(ui, ok, &mut done, &mut close);
                });
            }
            Some(Dialog::Part { kind, to, field }) => {
                egui::Modal::new(egui::Id::NULL.with("part_setup")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Part Definition"));
                    let have: Vec<u64> = self.layout.parts.iter().map(|p| p.kind).collect();
                    for k in PART_KINDS {
                        let sub =
                            matches!(k, part::LEADING_SUB_SUMMARY | part::TRAILING_SUB_SUMMARY);
                        let free = sub || !have.contains(&k);
                        ui.add_enabled_ui(free, |ui| {
                            ui.radio_value(kind, k, part_name(k));
                        });
                    }
                    let sub = matches!(
                        *kind,
                        part::LEADING_SUB_SUMMARY | part::TRAILING_SUB_SUMMARY
                    );
                    if sub {
                        ui.label(tr("When sorted by"));
                        if occurrences(ui, "part_to", to) {
                            *field = None;
                        }
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for (id, name) in fields_of(*to) {
                                    if ui.selectable_label(*field == Some(id), name).clicked() {
                                        *field = Some(id);
                                    }
                                }
                            });
                    }
                    buttons(ui, !sub || field.is_some(), &mut done, &mut close);
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
                    let ok = items.iter().any(|i| i.0 == *target);
                    buttons(ui, ok, &mut done, &mut close);
                });
            }
            Some(Dialog::NewLayout { name, to }) => {
                egui::Modal::new(egui::Id::NULL.with("new_layout")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("New Layout/Report"));
                    ui.label(tr("Show records from"));
                    occurrences(ui, "nl_to", to);
                    ui.horizontal(|ui| {
                        ui.label(tr("Layout Name"));
                        ui.text_edit_singleline(name);
                    });
                    buttons(ui, !name.trim().is_empty(), &mut done, &mut close);
                });
            }
            Some(Dialog::DeleteLayout) => {
                let name = self.layout.name.clone();
                egui::Modal::new(egui::Id::NULL.with("delete_layout")).show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(crate::i18n::delete_layout_question(&name));
                    ui.horizontal(|ui| {
                        if ui.button(tr("Delete")).clicked() {
                            done = true;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            }
            None => {}
        }
        if done {
            if let Some(d) = self.dialog.take() {
                self.finish_dialog(d, doc);
            }
        } else if close {
            self.dialog = None;
        }
        self.setup_dialog(ctx, doc, scripts);
    }

    fn setup_dialog(&mut self, ctx: &egui::Context, doc: &Doc, scripts: &[(u32, String)]) {
        let Some(s) = self.setup.as_mut() else {
            return;
        };
        let Some(ok) = s.show(ctx, doc, scripts) else {
            return;
        };
        if let Some(s) = self.setup.take().filter(|_| ok) {
            let before = self.layout.clone();
            if crate::layout_setup::apply(&mut self.layout, &s.layout) {
                self.undo.push(before);
                self.redo.clear();
                self.touched = None;
            }
        }
    }

    fn finish_dialog(&mut self, d: Dialog, doc: &Doc) {
        match d {
            Dialog::Field {
                rect,
                to,
                field: Some(f),
                label,
            } => self.insert_field(doc, rect, to, f, label),
            Dialog::Field { .. } => {}
            Dialog::Portal {
                rect,
                to,
                rows,
                fields,
            } => self.insert_portal(rect, to, rows, &fields),
            Dialog::TabControl { rect, tabs } => {
                let names: Vec<String> = tabs
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect();
                self.insert_tab_control(rect, &names);
            }
            Dialog::Part { kind, to, field } => {
                self.add_part(kind, field.map(|f| (to, f)));
            }
            Dialog::Button {
                rect,
                edit,
                label,
                script,
                target,
            } => self.set_button(rect, edit, &label, script, target),
            Dialog::NewLayout { name, to } => {
                self.pending = Some(Then::Op(LayoutOp::New {
                    name: name.trim().to_string(),
                    to,
                }));
            }
            Dialog::DeleteLayout => self.pending = Some(Then::Op(LayoutOp::Delete)),
        }
    }

    /// A field (and its label, to its left) over `rect`; inside a portal
    /// or tab panel when drawn there.
    pub fn insert_field(&mut self, doc: &Doc, rect: Bounds, to: u32, f: u32, label: bool) {
        self.checkpoint();
        let mut ids = Vec::new();
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
            ids.push(self.add(LayoutObject {
                kind: kind::TEXT,
                bounds: lb,
                style_raw: new_style(70),
                text: Some(name),
                ..Default::default()
            }));
        }
        ids.push(self.add(LayoutObject {
            kind: kind::FIELD,
            bounds: rect,
            style_raw: new_style(61),
            field: Some((0x104000 | to as u64, f as u64, 1)),
            ..Default::default()
        }));
        for id in &ids {
            self.place_inside(*id);
        }
        self.sel = ids.into_iter().collect();
    }

    /// A portal on `to` over `rect`, `rows` rows, with `fields` side by
    /// side in its first row.
    pub fn insert_portal(&mut self, rect: Bounds, to: u32, rows: u32, fields: &[u32]) {
        self.checkpoint();
        let pid = self.add(LayoutObject {
            kind: kind::PORTAL,
            bounds: rect.clone(),
            style_raw: new_style(83),
            portal: Some((0x104000 | to as u64, rows.max(1), 1)),
            ..Default::default()
        });
        let w = (rect.right - rect.left) / fields.len().max(1) as f64;
        let h = ((rect.bottom - rect.top) / rows.max(1) as f64).round();
        for (i, f) in fields.iter().enumerate() {
            let id = self.alloc_id();
            let x = (i as f64 * w).round();
            self.layout.objects.insert(
                0,
                LayoutObject {
                    id,
                    kind: kind::FIELD,
                    options: anchor::LEFT | anchor::TOP,
                    bounds: Bounds {
                        top: 0.0,
                        left: x,
                        bottom: h,
                        right: (x + w).round(),
                    },
                    style_raw: new_style(61),
                    field: Some((0x104000 | to as u64, *f as u64, 1)),
                    parent: Some(pid),
                    ..Default::default()
                },
            );
        }
        self.sel = BTreeSet::from([pid]);
    }

    /// A tab control over `rect` with one panel per name; the first is in
    /// front.
    pub fn insert_tab_control(&mut self, rect: Bounds, names: &[String]) {
        if names.is_empty() {
            return;
        }
        self.checkpoint();
        let tc = self.add(LayoutObject {
            kind: kind::TAB_CONTROL,
            bounds: rect.clone(),
            style_raw: new_style(81),
            ..Default::default()
        });
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let mut x = 0u32;
        let mut first = None;
        for n in names {
            let id = self.alloc_id();
            first.get_or_insert(id);
            let tw = (n.chars().count() as u32 * 8 + 32).max(60);
            // Panels follow their control in the list (behind it), in order.
            let at = self
                .layout
                .objects
                .iter()
                .position(|o| o.id == tc)
                .map_or(0, |p| p + 1 + (id - tc - 1) as usize);
            let at = at.min(self.layout.objects.len());
            self.layout.objects.insert(
                at,
                LayoutObject {
                    id,
                    kind: kind::TAB_PANEL,
                    options: anchor::ALL,
                    bounds: Bounds {
                        top: 0.0,
                        left: 0.0,
                        bottom: h,
                        right: w,
                    },
                    style_raw: new_style(82),
                    text: Some(n.clone()),
                    parent: Some(tc),
                    tab: Some((x, tw)),
                    ..Default::default()
                },
            );
            x += tw;
        }
        if let Some(o) = self.object_mut(tc) {
            o.active_child = first;
        }
        self.sel = BTreeSet::from([tc]);
    }

    fn set_button(
        &mut self,
        rect: Bounds,
        edit: Option<u64>,
        label: &str,
        script: bool,
        target: u32,
    ) {
        let step_id = if script {
            fmp12::scripts::id::PERFORM_SCRIPT
        } else {
            fmp12::scripts::id::GO_TO_LAYOUT
        };
        let step = fmp12::scripts::targeted_step(step_id, target);
        let tokens = fmp12::save::text_literal_tokens(label);
        self.checkpoint();
        match edit.and_then(|id| self.layout.objects.iter_mut().find(|o| o.id == id)) {
            Some(o) => {
                if o.button.as_ref().map(|s| (s.id, &s.params)) != Some((step.id, &step.params)) {
                    o.button = Some(step);
                }
                o.label = Some(tokens);
                o.text = Some(label.to_string());
            }
            None => {
                let id = self.add(LayoutObject {
                    kind: kind::BUTTON,
                    bounds: rect,
                    style_raw: new_style(80),
                    label: Some(tokens),
                    // FileMaker keeps the label text in the node too (key 1).
                    text: Some(label.to_string()),
                    button: Some(step),
                    ..Default::default()
                });
                self.place_inside(id);
            }
        }
    }
}

/// Grows (or shrinks, `dy` < 0) part `i`; the parts and the top-level
/// objects below follow.
fn resize_part(l: &mut Layout, i: usize, dy: f64) {
    let Some(p0) = l.parts.get(i) else { return };
    let old_bottom = p0.top + p0.height;
    let dy = dy.max(1.0 - p0.height);
    l.parts[i].height += dy;
    for (j, part) in l.parts.iter_mut().enumerate() {
        if j != i && part.top >= old_bottom - 0.5 {
            part.top += dy;
        }
    }
    for o in l.objects.iter_mut().filter(|o| o.parent.is_none()) {
        if o.bounds.top >= old_bottom - 0.5 {
            shift(&mut o.bounds, 0.0, dy);
        }
    }
}

/// Occurrences related to `to` (either side of a relationship), for
/// portals; all others when there is none.
fn related(doc: &Doc, to: u32) -> Vec<(u32, String)> {
    let ids: BTreeSet<u32> = doc
        .db
        .relationships
        .iter()
        .filter_map(|r| match (r.left == to, r.right == to) {
            (true, _) => Some(r.right),
            (_, true) => Some(r.left),
            _ => None,
        })
        .collect();
    doc.db
        .occurrences
        .iter()
        .filter(|o| o.id != to && (ids.is_empty() || ids.contains(&o.id)))
        .map(|o| (o.id, o.name.clone()))
        .collect()
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
    if objects::paint(painter, doc, l, o, lk, r, table, layout_to) {
        return;
    }
    let thin = Stroke::new(0.5, Color32::from_gray(150));
    match o.kind {
        // Line: from the top-left to the bottom-right corner.
        kind::LINE => {
            let (w, c) = lk.borders[0];
            let s = if w > 0.0 {
                Stroke::new(w, c)
            } else {
                Stroke::new(1.0, Color32::from_gray(60))
            };
            painter.line_segment([r.left_top(), r.right_bottom()], s);
            return;
        }
        kind::OVAL => {
            let (c, rad) = (r.center(), r.size() / 2.0);
            if let Some(bg) = lk.background {
                painter.add(egui::Shape::ellipse_filled(c, rad, bg));
            }
            let (w, col) = lk.borders[0];
            let s = if w > 0.0 { Stroke::new(w, col) } else { thin };
            painter.add(egui::Shape::ellipse_stroke(c, rad, s));
            return;
        }
        _ => lk.paint_box(painter, r),
    }
    let boxed = lk.background.is_some() || lk.borders.iter().any(|b| b.0 > 0.0);
    match o.kind {
        kind::FIELD => {
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
            // Control styles: an arrow for lists and menus, a box for
            // checkboxes, a circle for radio buttons, a calendar mark.
            let c = lk.content_rect(r);
            let mark = Stroke::new(1.0, Color32::from_gray(90));
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
                    painter.rect_stroke(b, 1.0, mark, StrokeKind::Inside);
                }
                4 => {
                    painter.circle_stroke(c.left_center() + Vec2::new(5.0, 0.0), 5.0, mark);
                }
                6 => {
                    let b = Rect::from_center_size(
                        egui::pos2(c.right() - 7.0, c.center().y),
                        Vec2::splat(10.0),
                    );
                    painter.rect_stroke(b, 1.0, mark, StrokeKind::Inside);
                    painter.hline(b.x_range(), b.top() + 3.0, mark);
                }
                _ => {}
            }
        }
        kind::TEXT => lk.paint_text(painter, r, o.text.as_deref().unwrap_or("")),
        kind::RECT if !boxed => {
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
        }
        kind::BUTTON => {
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
        kind::PORTAL => {
            let rows = o.portal.map_or(1, |p| p.1.max(1));
            let h = r.height() / rows as f32;
            for i in 1..rows {
                painter.hline(r.x_range(), r.top() + h * i as f32, thin);
            }
            if !boxed {
                painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            }
        }
        kind::TAB_CONTROL => {
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
            if !boxed {
                painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            }
        }
        _ if !boxed => {
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
        }
        _ => {}
    }
}

/// Layouts of `doc.db` that have a table occurrence (folders excluded).
pub fn reload_layouts(d: &mut Doc) {
    d.layouts =
        d.db.layouts
            .iter()
            .filter(|l| l.table_occurrence != 0)
            .cloned()
            .collect();
}

/// Runs a Layouts menu command on the file: the document's layouts are
/// written first, then the layout is added, duplicated or deleted
/// (`fmp12::schema_edit`). Returns the index (in `Doc::layouts`) of the
/// layout to show.
pub fn run_layout_op(d: &mut Doc, current: usize, op: &LayoutOp) -> Result<usize, String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).map_err(|x| e(&x))?;
    d.file = fmp12::File::from_bytes(bytes).map_err(|x| e(&x))?;
    let cur = d.layouts.get(current).cloned().ok_or("no layout")?;
    let name = |base: &str, d: &Doc| {
        std::iter::once(base.to_string())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|n| d.db.layouts.iter().all(|l| &l.name != n))
            .unwrap_or_default()
    };
    let target = match op {
        LayoutOp::New { name: n, to } => {
            let n = name(n, d);
            let id =
                d.db.edit_schema(&mut d.file, |ed| ed.new_layout(*to, &n))
                    .map_err(|x| e(&x))?;
            Some(id)
        }
        LayoutOp::Duplicate => {
            let n = name(&format!("{} {}", cur.name, tr("Copy")), d);
            let id =
                d.db.edit_schema(&mut d.file, |ed| ed.duplicate_layout(cur.id, &n))
                    .map_err(|x| e(&x))?;
            Some(id)
        }
        LayoutOp::Delete => {
            if d.layouts.len() <= 1 {
                return Err(tr("A file needs at least one layout.").into());
            }
            d.db.edit_schema(&mut d.file, |ed| ed.delete_layout(cur.id))
                .map_err(|x| e(&x))?;
            None
        }
    };
    reload_layouts(d);
    Ok(match target {
        Some(id) => d.layouts.iter().position(|l| l.id == id).unwrap_or(0),
        None => current.min(d.layouts.len().saturating_sub(1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FileMaker starter template from a local install, if any.
    pub(crate) fn template(name: &str) -> Option<Doc> {
        let p = std::path::PathBuf::from(format!(
            r"C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\{name}\en.fmp12"
        ));
        let file = fmp12::File::open(p).ok()?;
        let db = engine::Database::load(&file).ok()?;
        let layouts = db
            .layouts
            .iter()
            .filter(|l| l.table_occurrence != 0)
            .cloned()
            .collect();
        Some(Doc {
            name: name.into(),
            db,
            layouts,
            file,
            path: None,
            triggers: Default::default(),
        })
    }

    fn doc() -> Option<Doc> {
        template("Contacts")
    }

    fn bounds(ds: &Designer, id: u64) -> (f64, f64, f64, f64) {
        let b = &ds.object(id).unwrap().bounds;
        (b.top, b.left, b.bottom, b.right)
    }

    fn designer(d: &Doc, name: &str) -> Designer {
        let i = d.layouts.iter().position(|l| l.name == name).unwrap();
        Designer::new(d, i)
    }

    /// Saves the document's layouts and reads them back.
    fn saved(d: &Doc) -> (fmp12::File, Vec<Layout>) {
        let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
        let f = fmp12::File::from_bytes(bytes).unwrap();
        let back = fmp12::layouts::layouts(&f).unwrap();
        (f, back)
    }

    #[test]
    fn edit_with_the_mouse_and_save() {
        let Some(mut d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        // Field 9 (Last Name) spans (112, 366)–(156, 632): click and drag it.
        let p = egui::pos2(400.0, 130.0);
        ds.press(p, false, false, None);
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
        ds.press(br, false, false, None);
        ds.drag_to(br + Vec2::new(-42.0, 4.0));
        ds.release();
        assert_eq!(bounds(&ds, 9), (132.0, 376.0, 180.0, 600.0));
        // Shift-click adds field 8, marquee selects fields 10 and 12.
        ds.press(egui::pos2(700.0, 120.0), true, false, None);
        ds.release();
        assert_eq!(ds.sel, BTreeSet::from([8, 9]));
        ds.press(egui::pos2(5.0, 108.0), false, false, None);
        ds.drag_to(egui::pos2(365.0, 160.0));
        ds.release();
        assert!(ds.sel.contains(&10) && ds.sel.contains(&12) && !ds.sel.contains(&9));
        // Body (104–164) grows by 20 from its boundary in the part gutter.
        ds.sel.clear();
        ds.press(egui::pos2(-10.0, 164.0), false, true, None);
        ds.drag_to(egui::pos2(-10.0, 184.0));
        ds.release();
        let body = ds.layout.parts.iter().find(|p| p.kind == 4).unwrap();
        assert_eq!(body.height, 80.0);
        // A rectangle drawn with the tool, deleted, then restored by undo.
        ds.tool = Tool::Rect;
        ds.press(egui::pos2(700.0, 50.0), false, false, None);
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
        let (_, back) = saved(&d);
        let l = back.iter().find(|l| l.name == "Contact List").unwrap();
        let o9 = l.objects.iter().find(|o| o.id == 9).unwrap();
        assert_eq!(o9.bounds, ds.object(9).unwrap().bounds);
        assert_eq!(
            l.parts.iter().map(|p| p.height).collect::<Vec<_>>(),
            ds.layout.parts.iter().map(|p| p.height).collect::<Vec<_>>()
        );
        assert_eq!(l.objects.iter().find(|o| o.id == r).unwrap().kind, 5);
    }

    #[test]
    fn arrange_commands() {
        let Some(d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        // Fields 10 (96–362), 9 (366–632), 8 (636–976) on one row.
        ds.sel = BTreeSet::from([8, 9, 10]);
        ds.nudge(0.0, 0.0);
        ds.move_by(9, 0.0, 7.0);
        ds.align(Align::Top);
        assert_eq!(bounds(&ds, 9).0, 112.0);
        ds.align(Align::Left);
        assert!([8, 9, 10].iter().all(|i| bounds(&ds, *i).1 == 96.0));
        ds.undo();
        ds.move_by(9, 50.0, 0.0);
        ds.distribute(true);
        // Gaps between 10, 9 and 8 are equal again (first and last stay).
        let (a, b, c) = (bounds(&ds, 10), bounds(&ds, 9), bounds(&ds, 8));
        assert_eq!((b.1 - a.3).round(), (c.1 - b.3).round());
        ds.resize_to(true, true, false);
        let w = |i| {
            let b = bounds(&ds, i);
            b.3 - b.1
        };
        assert_eq!(w(10), 340.0);
        assert_eq!(w(9), 340.0);
        // Z-order: send 8 to the back, bring it to the front.
        ds.sel = BTreeSet::from([8]);
        ds.arrange_z(false, true);
        assert_eq!(ds.layout.objects.last().unwrap().id, 8);
        ds.arrange_z(true, true);
        assert_eq!(ds.layout.objects[0].id, 8);
        // Group then ungroup: members keep their place.
        ds.sel = BTreeSet::from([9, 10]);
        let before = (ds.layout.absolute(ds.object(9).unwrap()), bounds(&ds, 10));
        ds.group();
        let g = *ds.sel.iter().next().unwrap();
        assert_eq!(ds.object(g).unwrap().kind, kind::GROUP);
        assert_eq!(ds.object(9).unwrap().parent, Some(g));
        assert_eq!(ds.layout.absolute(ds.object(9).unwrap()), before.0);
        // A click on a member selects the group.
        ds.press(egui::pos2(400.0, 130.0), false, false, None);
        ds.release();
        assert_eq!(ds.sel, BTreeSet::from([g]));
        ds.ungroup();
        assert!(ds.object(g).is_none());
        assert_eq!(bounds(&ds, 10), before.1);
        // Lock: a locked object neither moves nor goes away.
        ds.sel = BTreeSet::from([10]);
        ds.set_locked(true);
        ds.nudge(5.0, 0.0);
        ds.delete_selection();
        assert_eq!(bounds(&ds, 10), before.1);
    }

    #[test]
    fn clipboard_and_duplicate() {
        let Some(mut d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        ds.sel = BTreeSet::from([9]);
        ds.duplicate();
        let dup = *ds.sel.iter().next().unwrap();
        assert_ne!(dup, 9);
        assert_eq!(bounds(&ds, dup), (122.0, 376.0, 166.0, 642.0));
        assert_eq!(ds.object(dup).unwrap().field, ds.object(9).unwrap().field);
        // Copy on one layout, paste on another: same place.
        ds.sel = BTreeSet::from([3]);
        ds.copy();
        let mut other = designer(&d, "Contact Details");
        let n = other.layout.objects.len();
        other.paste();
        assert_eq!(other.layout.objects.len(), n + 1);
        let p = other.layout.objects[0].clone();
        assert_eq!(p.text.as_deref(), Some("Company"));
        ds.apply(&mut d);
        let (_, back) = saved(&d);
        let l = back.iter().find(|l| l.name == "Contact List").unwrap();
        assert!(l.objects.iter().any(|o| o.id == dup && o.field.is_some()));
    }

    #[test]
    fn parts_added_and_deleted_are_saved() {
        let Some(mut d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        let body_top = ds.layout.parts.iter().find(|p| p.kind == 4).unwrap().top;
        // A trailing grand summary and a footer go below the body.
        ds.add_part(part::TRAILING_GRAND_SUMMARY, None);
        ds.add_part(part::FOOTER, None);
        ds.add_part(part::FOOTER, None);
        let kinds: Vec<u64> = ds.layout.parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [12, 1, 3, 4, 6, 7]);
        // A title header goes on top and pushes everything down by 40.
        ds.add_part(part::TITLE_HEADER, None);
        assert_eq!(ds.layout.parts[1].kind, part::TITLE_HEADER);
        assert_eq!(
            ds.layout.parts.iter().find(|p| p.kind == 4).unwrap().top,
            body_top + 40.0
        );
        assert_eq!(bounds(&ds, 9).0, 152.0);
        ds.delete_part(1);
        assert_eq!(bounds(&ds, 9).0, 112.0);
        ds.add_part(part::LEADING_GRAND_SUMMARY, None);
        ds.apply(&mut d);
        let (f, back) = saved(&d);
        let l = back.iter().find(|l| l.name == "Contact List").unwrap();
        let kinds: Vec<u64> = l.parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [12, 1, 2, 3, 4, 6, 7]);
        let tops: Vec<f64> = l.parts.iter().map(|p| p.top).collect();
        assert_eq!(
            tops,
            ds.layout.parts.iter().map(|p| p.top).collect::<Vec<_>>()
        );
        assert_eq!(
            l.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6, 7]
        );
        // The sub-summary keeps its break field.
        assert!(l.parts[3].break_field.is_some());
        // Saving again (with the new part ids) changes nothing.
        let mut d2 = d;
        d2.file = f;
        d2.db.layouts = back.clone();
        let (_, again) = saved(&d2);
        assert_eq!(again, back);
    }

    #[test]
    fn portal_tab_control_and_oval_are_saved() {
        let Some(mut d) = doc() else { return };
        let mut ds = designer(&d, "Contact Details");
        let to = (ds.layout.table_occurrence & 0xFFF) as u32;
        let rel = related(&d, to);
        let (pto, _) = rel[0].clone();
        let fields: Vec<u32> =
            d.db.table_of(pto)
                .unwrap()
                .fields
                .iter()
                .take(2)
                .map(|f| f.id)
                .collect();
        let rect = |t: f64, l: f64, b: f64, r: f64| Bounds {
            top: t,
            left: l,
            bottom: b,
            right: r,
        };
        ds.insert_portal(rect(100.0, 700.0, 220.0, 1000.0), pto, 4, &fields);
        let portal = *ds.sel.iter().next().unwrap();
        ds.insert_tab_control(
            rect(240.0, 700.0, 400.0, 1000.0),
            &["One".into(), "Two".into()],
        );
        let tc = *ds.sel.iter().next().unwrap();
        // A field drawn inside the front panel goes onto it.
        let first = ds.object(tc).unwrap().active_child.unwrap();
        ds.insert_field(&d, rect(300.0, 760.0, 320.0, 900.0), to, 1, false);
        let f = *ds.sel.iter().next().unwrap();
        assert_eq!(ds.object(f).unwrap().parent, Some(first));
        assert_eq!(bounds(&ds, f), (60.0, 60.0, 80.0, 200.0));
        ds.tool = Tool::Oval;
        ds.create(egui::pos2(20.0, 450.0), egui::pos2(80.0, 490.0));
        let oval = *ds.sel.iter().next().unwrap();
        ds.apply(&mut d);
        let (f2, back) = saved(&d);
        let l = back.iter().find(|l| l.name == "Contact Details").unwrap();
        let get = |id: u64| l.objects.iter().find(|o| o.id == id).unwrap();
        assert_eq!(get(portal).kind, 9);
        assert_eq!(get(portal).portal, Some((0x104000 | pto as u64, 4, 1)));
        let kids: Vec<&LayoutObject> = l
            .objects
            .iter()
            .filter(|o| o.parent == Some(portal))
            .collect();
        assert_eq!(kids.len(), 2);
        let panels: Vec<&LayoutObject> =
            l.objects.iter().filter(|o| o.parent == Some(tc)).collect();
        assert_eq!(panels.len(), 2);
        assert_eq!(
            panels
                .iter()
                .filter_map(|p| p.text.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["One".to_string(), "Two".to_string()])
        );
        assert_eq!(get(f).parent, Some(first));
        assert_eq!(get(oval).kind, 7);
        // The new field is in the tab order and in the body's object list.
        let tabs = fmp12::write::entries(&f2)
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.path.len() == 4 && e.path[3] == [21] && e.path[2] == fmp12::save::path_elem(l.id)
            })
            .count();
        assert!(tabs > 0);
    }

    #[test]
    fn styles_written_as_local_properties() {
        let Some(mut d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        let ids = [3u64];
        ds.set_prop(&ids, "size", prop::FONT_SIZE, Some(pt(20.0)));
        ds.set_prop(&ids, "bold", prop::FONT_WEIGHT, Some(theme::Value::Enum(2)));
        ds.set_prop(&ids, "align", prop::TEXT_ALIGN, Some(theme::Value::Enum(3)));
        ds.set_prop(&ids, "u", prop::UNDERLINE, Some(theme::Value::Enum(1)));
        ds.apply(&mut d);
        let (_, back) = saved(&d);
        let l = back.iter().find(|l| l.name == "Contact List").unwrap();
        let o = l.objects.iter().find(|o| o.id == 3).unwrap();
        let s = ObjectStyle::parse(&o.style_raw);
        let p = &s.local[&(SELF, NORMAL)];
        assert_eq!(p.length(prop::FONT_SIZE).unwrap().value, 20.0);
        assert_eq!(p.enum_value(prop::FONT_WEIGHT), Some(2));
        assert_eq!(p.enum_value(prop::TEXT_ALIGN), Some(3));
        assert_eq!(p.enum_value(prop::UNDERLINE), Some(1));
        let th = d.db.themes.iter().find(|t| t.number == l.theme);
        assert_eq!(look::resolve(th, &s).font_size, 20.0);
        assert_eq!(look::resolve(th, &s).align, 2);
    }

    #[test]
    fn guides_and_grid_snap() {
        let Some(d) = doc() else { return };
        let mut ds = designer(&d, "Contact List");
        // A guide dragged from the top ruler to y = 200.
        ds.press(egui::pos2(300.0, -10.0), false, false, Some(false));
        ds.drag_to(egui::pos2(300.0, 200.0));
        ds.release();
        assert_eq!(
            ds.guides,
            [Guide {
                vertical: false,
                at: 200.0
            }]
        );
        // Field 9 (top 112, bottom 156) dragged by 42: its bottom snaps to 200.
        ds.press(egui::pos2(400.0, 130.0), false, false, None);
        ds.drag_to(egui::pos2(400.0, 172.0));
        ds.release();
        assert_eq!(bounds(&ds, 9).2, 200.0);
        // Snap to grid: the top-left corner lands on the 12-point grid.
        ds.guides.clear();
        ds.snap_grid = true;
        ds.press(egui::pos2(400.0, 160.0), false, false, None);
        ds.drag_to(egui::pos2(405.0, 163.0));
        ds.release();
        let b = bounds(&ds, 9);
        assert_eq!((b.0 % 12.0, b.1 % 12.0), (0.0, 0.0));
    }

    #[test]
    fn layouts_menu_writes_the_file() {
        let Some(mut d) = doc() else { return };
        let n = d.layouts.len();
        let i = d
            .layouts
            .iter()
            .position(|l| l.name == "Contact List")
            .unwrap();
        let to = (d.layouts[i].table_occurrence & 0xFFF) as u32;
        let j = run_layout_op(
            &mut d,
            i,
            &LayoutOp::New {
                name: "Report".into(),
                to,
            },
        )
        .unwrap();
        assert_eq!(d.layouts[j].name, "Report");
        assert_eq!(d.layouts.len(), n + 1);
        let k = run_layout_op(&mut d, i, &LayoutOp::Duplicate).unwrap();
        assert_eq!(d.layouts[k].name, format!("Contact List {}", tr("Copy")));
        assert_eq!(d.layouts[k].objects.len(), d.layouts[i].objects.len());
        // The file itself has them.
        let names: Vec<String> = fmp12::layouts::layouts(&d.file)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert!(names.contains(&"Report".to_string()));
        run_layout_op(&mut d, j, &LayoutOp::Delete).unwrap();
        assert_eq!(d.layouts.len(), n + 1);
        assert!(d.layouts.iter().all(|l| l.name != "Report"));
        // A rename through Layout Setup reaches the name index.
        let mut ds = Designer::new(&d, i);
        ds.layout.name = "Contacts Overview".into();
        ds.apply(&mut d);
        let (f, back) = saved(&d);
        assert!(back.iter().any(|l| l.name == "Contacts Overview"));
        let key = fmp12::collate::name_key("Contacts Overview").unwrap();
        let found = fmp12::write::entries(&f).unwrap().iter().any(|e| {
            e.path == [vec![4], vec![1], vec![1]]
                && fmp12::chunk::parse(&e.item).ok().is_some_and(|c| {
                    matches!(c.first(), Some(fmp12::Chunk::KvLong { key: k, .. }) if *k == key.as_slice())
                })
        });
        assert!(found, "name index entry");
    }

    /// The file `tools/oracle/layout_check.ps1` opens in FileMaker: a copy
    /// of Contacts whose layouts BaseCraft edited through the designer.
    /// Written when `BASECRAFT_LAYOUT_DEMO=<out.fmp12>` is set.
    #[test]
    fn layout_demo_file() {
        let Some(out) = std::env::var_os("BASECRAFT_LAYOUT_DEMO") else {
            return;
        };
        let Some(mut d) = doc() else { return };
        let b = |t: f64, l: f64, bt: f64, r: f64| Bounds {
            top: t,
            left: l,
            bottom: bt,
            right: r,
        };
        let list = d
            .layouts
            .iter()
            .position(|l| l.name == "Contact List")
            .unwrap();
        let to = (d.layouts[list].table_occurrence & 0xFFF) as u32;
        // Layouts menu: a new layout, a duplicate (renamed), one deleted.
        let gone = run_layout_op(
            &mut d,
            list,
            &LayoutOp::New {
                name: "Scratch".into(),
                to,
            },
        )
        .unwrap();
        run_layout_op(&mut d, gone, &LayoutOp::Delete).unwrap();
        let dup = run_layout_op(&mut d, list, &LayoutOp::Duplicate).unwrap();
        let mut ds = Designer::new(&d, dup);
        ds.layout.name = "Contact List 2".into();
        // Contact List 2: a footer and a sub-summary by Company, a bold title.
        ds.add_part(part::FOOTER, None);
        ds.add_part(part::TRAILING_GRAND_SUMMARY, None);
        ds.set_prop(&[3], "size", prop::FONT_SIZE, Some(pt(18.0)));
        ds.set_prop(&[3], "bold", prop::FONT_WEIGHT, Some(theme::Value::Enum(2)));
        ds.apply(&mut d);
        let i = run_layout_op(
            &mut d,
            list,
            &LayoutOp::New {
                name: "BaseCraft Demo".into(),
                to,
            },
        )
        .unwrap();
        let mut ds = Designer::new(&d, i);
        let body = ds
            .layout
            .parts
            .iter()
            .find(|p| p.kind == part::BODY)
            .unwrap()
            .clone();
        let y = body.top;
        // Taller body, a title header and a leading grand summary.
        let bi = ds
            .layout
            .parts
            .iter()
            .position(|p| p.kind == part::BODY)
            .unwrap();
        resize_part(&mut ds.layout, bi, 500.0);
        ds.add_part(part::LEADING_GRAND_SUMMARY, None);
        let y = y + 40.0;
        // Fields with labels; the second styled.
        ds.insert_field(&d, b(y + 10.0, 130.0, y + 54.0, 330.0), to, 1, true);
        ds.insert_field(&d, b(y + 60.0, 130.0, y + 104.0, 330.0), to, 2, true);
        let last = *ds.sel.iter().max().unwrap();
        ds.set_prop(&[last], "size", prop::FONT_SIZE, Some(pt(16.0)));
        ds.set_prop(
            &[last],
            "bold",
            prop::FONT_WEIGHT,
            Some(theme::Value::Enum(2)),
        );
        ds.set_prop(
            &[last],
            "italic",
            prop::FONT_STYLE,
            Some(theme::Value::Enum(2)),
        );
        ds.set_prop(
            &[last],
            "align",
            prop::TEXT_ALIGN,
            Some(theme::Value::Enum(2)),
        );
        let red = theme::Value::Color(Rgba {
            r: 0.8,
            g: 0.1,
            b: 0.1,
            a: 1.0,
        });
        ds.set_prop(&[last], "color", prop::COLOR, Some(red));
        // A drop-down calendar? Contacts has no date field: a pop-up menu.
        ds.insert_field(&d, b(y + 110.0, 130.0, y + 154.0, 330.0), to, 4, true);
        let title = *ds.sel.iter().max().unwrap();
        if let Some(o) = ds.object_mut(title) {
            o.display = 2;
            o.style_raw = theme::with_object_type(&o.style_raw, 63);
        }
        // Portal on a related occurrence, 3 rows, 2 fields.
        let (pto, _) = related(&d, to)[0].clone();
        let pf: Vec<u32> =
            d.db.table_of(pto)
                .unwrap()
                .fields
                .iter()
                .take(2)
                .map(|f| f.id)
                .collect();
        ds.insert_portal(b(y + 170.0, 20.0, y + 302.0, 420.0), pto, 3, &pf);
        // Tab control with two panels and a field on the first.
        ds.insert_tab_control(
            b(y + 320.0, 20.0, y + 470.0, 420.0),
            &["Main".into(), "More".into()],
        );
        ds.insert_field(&d, b(y + 380.0, 140.0, y + 424.0, 400.0), to, 5, true);
        // Button running the first script.
        let script = fmp12::scripts::scripts(&d.file)
            .unwrap()
            .into_iter()
            .find(|s| !s.is_folder() && !s.steps.is_empty())
            .unwrap();
        ds.set_button(
            b(y + 10.0, 450.0, y + 54.0, 600.0),
            None,
            "Run script",
            true,
            script.id,
        );
        // Text, shapes; a group; a locked rectangle; right-anchored text.
        ds.tool = Tool::Text;
        ds.create(
            egui::pos2(450.0, (y + 60.0) as f32),
            egui::pos2(650.0, (y + 84.0) as f32),
        );
        ds.end_text_edit();
        let t = *ds.sel.iter().next().unwrap();
        ds.object_mut(t).unwrap().text = Some("Hello from BaseCraft".into());
        ds.set_prop(&[t], "u", prop::UNDERLINE, Some(theme::Value::Enum(1)));
        if let Some(o) = ds.object_mut(t) {
            o.options |= anchor::RIGHT;
        }
        for (tool, r) in [
            (Tool::Rect, (450.0, y + 100.0, 560.0, y + 160.0)),
            (Tool::Oval, (580.0, y + 100.0, 680.0, y + 160.0)),
            (Tool::Line, (450.0, y + 180.0, 680.0, y + 180.0)),
        ] {
            ds.tool = tool;
            ds.create(
                egui::pos2(r.0 as f32, r.1 as f32),
                egui::pos2(r.2 as f32, r.3 as f32),
            );
            let id = *ds.sel.iter().next().unwrap();
            if tool != Tool::Line {
                let fill = theme::Value::Color(Rgba {
                    r: 0.6,
                    g: 0.8,
                    b: 1.0,
                    a: 1.0,
                });
                ds.set_prop(&[id], "fill", prop::BACKGROUND_COLOR, Some(fill));
            }
            if tool == Tool::Rect {
                let r = Length {
                    value: 10.0,
                    unit: Unit::Pt,
                };
                ds.set_styles(&[id], "radius", |s| {
                    let mut s = s.to_vec();
                    for i in 0..4 {
                        s = theme::with_local_prop(
                            &s,
                            SELF,
                            NORMAL,
                            prop::BORDER_RADIUS + i,
                            Some(&theme::Value::Pair(r, r)),
                        );
                    }
                    s
                });
                ds.sel = BTreeSet::from([id]);
                ds.set_locked(true);
            }
        }
        // Group the two texts made for labels? Group the oval and the line.
        let oval = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::OVAL)
            .unwrap()
            .id;
        let line = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::LINE)
            .unwrap()
            .id;
        ds.sel = BTreeSet::from([oval, line]);
        ds.group();
        ds.layout.width = ds.layout.width.max(720.0);
        ds.apply(&mut d);
        let bytes = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
        std::fs::write(out, bytes).unwrap();
    }
}
