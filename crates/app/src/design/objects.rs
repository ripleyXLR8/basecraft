//! Layout mode for popovers, slide controls, button bars and web viewers
//! (Insert menu and tools), data formatting, placeholders, tooltips and
//! "hide object when" (Inspector), conditional formatting (Format menu)
//! and the tab order (Arrange › Set Tab Order…).

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Rect, Stroke, StrokeKind, Vec2};
use fmp12::charts::{Chart, Kind, Source};
use fmp12::formats::{Format, num, time};
use fmp12::layouts::{self as ly, Bounds, Condition, Layout, LayoutObject, kind, op};
use fmp12::theme::{self, NORMAL, SELF};

use super::{Designer, new_style, rgba};
use crate::Doc;
use crate::i18n::tr;
use crate::look::Look;

/// Style types (FORMAT.md §12) of the objects added here.
const POPOVER_BUTTON_TYPE: u8 = 80;
const POPOVER_TYPE: u8 = 89;
const SLIDE_TYPE: u8 = 91;
const SLIDE_PANEL_TYPE: u8 = 92;
const WEB_TYPE: u8 = 86;
const BAR_TYPE: u8 = 78;
const SEGMENT_TYPE: u8 = 79;
/// Web viewer options of a new one in FileMaker 26 (interaction allowed,
/// progress bar, status messages, encoded URL).
const WEB_FLAGS: u64 = 32781;
/// Seen on new popover buttons and button bar segments (meaning unknown).
const NEW_BUTTON_BIT: u64 = 0x8;

/// Dialogs and editing buffers of this module.
#[derive(Default)]
pub(super) struct Extra {
    /// Conditional Formatting: object, conditions being edited, selected row.
    cond: Option<(u64, Vec<CondRow>, usize)>,
    /// Set Tab Order: the order being edited, the selected row.
    tabs: Option<(Vec<u64>, usize)>,
    /// Web Viewer Setup: rectangle (or the object edited), address.
    web: Option<(Bounds, Option<u64>, String)>,
    /// Calculation text being typed per (object, slot).
    calcs: BTreeMap<(u64, &'static str), String>,
    /// Chart Setup: the chart being edited.
    chart: Option<ChartEdit>,
}

/// Chart Setup's fields: the calculations as text, the rest as read.
struct ChartEdit {
    id: u64,
    chart: Chart,
    title: String,
    x_title: String,
    y_title: String,
    x: String,
    /// (value, name) per Y series.
    series: Vec<(String, String)>,
    size: String,
    error: Option<String>,
}

const CHART_KINDS: [(Kind, &str); 10] = [
    (Kind::Column, "Column"),
    (Kind::Bar, "Bar"),
    (Kind::Line, "Line"),
    (Kind::Area, "Area"),
    (Kind::Pie, "Pie"),
    (Kind::PositiveNegative, "Positive/Negative"),
    (Kind::StackedColumn, "Stacked Column"),
    (Kind::StackedBar, "Stacked Bar"),
    (Kind::Bubble, "Bubble"),
    (Kind::Scatter, "Scatter"),
];

/// Web Viewer Setup's options (FORMAT.md §10, [`ly::web`]).
const WEB_OPTIONS: [(u64, &str); 6] = [
    (
        ly::web::INTERACTION,
        "Allow interaction with web viewer content",
    ),
    (
        ly::web::JAVASCRIPT,
        "Allow JavaScript to perform FileMaker scripts",
    ),
    (ly::web::FIND_MODE, "Display content in Find mode"),
    (ly::web::PROGRESS_BAR, "Display progress bar"),
    (ly::web::STATUS, "Display status messages"),
    (ly::web::ENCODE_URL, "Automatically encode URL"),
];

impl ChartEdit {
    fn read(doc: &Doc, id: u64, c: &Chart) -> ChartEdit {
        let t = |x: &Option<Vec<u8>>| x.as_ref().map(|t| calc_text(doc, t)).unwrap_or_default();
        ChartEdit {
            id,
            chart: c.clone(),
            title: t(&c.title),
            x_title: t(&c.x_title),
            y_title: t(&c.y_title),
            x: t(&c.x),
            series: c
                .y
                .iter()
                .map(|s| (calc_text(doc, &s.value), t(&s.name)))
                .collect(),
            size: t(&c.size),
            error: None,
        }
    }

    /// The chart with the calculations compiled (an empty one is removed;
    /// series values and a bubble's size are required).
    fn build(&self, doc: &Doc) -> Result<Chart, String> {
        let opt = |s: &str| -> Result<Option<Vec<u8>>, String> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                compile(doc, s).map(Some)
            }
        };
        let req = |s: &str| -> Result<Vec<u8>, String> {
            opt(s)?.ok_or_else(|| tr("Specify a calculation for each series.").to_string())
        };
        let mut c = self.chart.clone();
        c.title = opt(&self.title)?;
        c.x_title = opt(&self.x_title)?;
        c.y_title = opt(&self.y_title)?;
        c.x = opt(&self.x)?;
        for ((v, n), s) in self.series.iter().zip(c.y.iter_mut()) {
            s.value = req(v)?;
            s.name = opt(n)?;
        }
        if c.size.is_some() {
            c.size = Some(req(&self.size)?);
        }
        Ok(c)
    }
}

/// A condition as the dialog edits it.
#[derive(Clone)]
struct CondRow {
    op: u64,
    formula: String,
    value1: String,
    value2: String,
    color: Option<Color32>,
    fill: Option<Color32>,
    bold: bool,
    italic: bool,
    underline: bool,
    /// The condition as read (kept when nothing changed).
    orig: Option<Condition>,
}

const OPS: [(u64, &str); 21] = [
    (op::FORMULA, "Formula is"),
    (op::BETWEEN, "Value is between"),
    (op::NOT_BETWEEN, "Value is not between"),
    (op::EQUAL, "Value is equal to"),
    (op::NOT_EQUAL, "Value is not equal to"),
    (op::GREATER, "Value is greater than"),
    (op::LESS, "Value is less than"),
    (op::GREATER_EQUAL, "Value is greater than or equal to"),
    (op::LESS_EQUAL, "Value is less than or equal to"),
    (op::CONTAINING, "Value is containing"),
    (op::NOT_CONTAINING, "Value is not containing"),
    (op::BEGINNING, "Value is beginning with"),
    (op::ENDING, "Value is ending with"),
    (op::EMPTY, "Value is empty"),
    (op::BEFORE_TODAY, "Value is before today"),
    (op::TODAY, "Value is today"),
    (op::AFTER_TODAY, "Value is after today"),
    (op::MORE_DAYS_AFTER, "Value is more than x days after"),
    (op::MORE_DAYS_BEFORE, "Value is more than x days before"),
    (op::LESS_DAYS_AFTER, "Value is less than x days after"),
    (op::LESS_DAYS_BEFORE, "Value is less than x days before"),
];

fn names(doc: &Doc) -> script::text::Names<'_> {
    script::text::Names {
        db: &doc.db,
        scripts: &[],
    }
}

/// Calculation text of a token stream (context 0: `TO::Field`).
pub(super) fn calc_text(doc: &Doc, tokens: &[u8]) -> String {
    fmp12::calc::decompile(tokens, 0, &names(doc)).unwrap_or_default()
}

fn compile(doc: &Doc, src: &str) -> Result<Vec<u8>, String> {
    names(doc)
        .compile(src)
        .map(|c| c.0)
        .map_err(|e| e.to_string())
}

fn color32(c: theme::Rgba) -> Color32 {
    let f = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(f(c.r), f(c.g), f(c.b))
}

impl CondRow {
    fn read(doc: &Doc, k: &Condition) -> CondRow {
        let rules = theme::rules(
            fmp12::pb::Msg::parse(&k.style())
                .and_then(|m| m.bytes(1))
                .unwrap_or_default(),
        );
        let props = rules.get(&(SELF, NORMAL)).cloned().unwrap_or_default();
        use theme::PropsExt;
        CondRow {
            op: k.op,
            formula: if k.op == op::FORMULA {
                k.calc
                    .as_ref()
                    .map(|t| calc_text(doc, t))
                    .unwrap_or_default()
            } else {
                String::new()
            },
            value1: k.value1.clone(),
            value2: k.value2.clone(),
            color: props.color(theme::prop::COLOR).map(color32),
            fill: props.color(theme::prop::BACKGROUND_COLOR).map(color32),
            bold: props.enum_value(theme::prop::FONT_WEIGHT) == Some(2),
            italic: props.enum_value(theme::prop::FONT_STYLE) == Some(2),
            underline: props
                .enum_value(theme::prop::UNDERLINE)
                .is_some_and(|u| u > 0),
            orig: Some(k.clone()),
        }
    }

    fn new() -> CondRow {
        CondRow {
            op: op::EQUAL,
            formula: String::new(),
            value1: String::new(),
            value2: String::new(),
            color: Some(Color32::from_rgb(255, 0, 0)),
            fill: None,
            bold: false,
            italic: false,
            underline: false,
            orig: None,
        }
    }

    /// The condition, its style built as FileMaker's dialog does (local
    /// properties of `.self` in the normal state); its formula compiled
    /// (for a value test, the formula FileMaker writes on `Self`).
    fn build(&self, doc: &Doc) -> Result<Condition, String> {
        let mut style: Vec<u8> = Vec::new();
        let mut set = |id: u32, v: Option<theme::Value>| {
            style = theme::with_local_prop(&style, SELF, NORMAL, id, v.as_ref());
        };
        set(
            theme::prop::COLOR,
            self.color.map(|c| theme::Value::Color(rgba(c))),
        );
        set(
            theme::prop::BACKGROUND_COLOR,
            self.fill.map(|c| theme::Value::Color(rgba(c))),
        );
        if self.bold {
            set(theme::prop::FONT_WEIGHT, Some(theme::Value::Enum(2)));
        }
        if self.italic {
            set(theme::prop::FONT_STYLE, Some(theme::Value::Enum(2)));
        }
        if self.underline {
            set(theme::prop::UNDERLINE, Some(theme::Value::Enum(1)));
        }
        let src = if self.op == op::FORMULA {
            self.formula.clone()
        } else {
            ly::value_formula(self.op, &self.value1, &self.value2).unwrap_or_default()
        };
        let calc = compile(doc, &src)?;
        let base = self.orig.clone().unwrap_or_default();
        let mut k = Condition {
            op: self.op,
            calc: Some(calc),
            value1: if self.op == op::FORMULA {
                String::new()
            } else {
                self.value1.clone()
            },
            value2: if self.op == op::FORMULA {
                String::new()
            } else {
                self.value2.clone()
            },
            format: base.format.clone(),
        };
        k.format = k.with_style(&style);
        Ok(k)
    }
}

/// Number format choices of the Inspector: name, and the option bits set
/// (the others keep theirs).
const NUMBER_FORMATS: [&str; 6] = [
    "As entered",
    "General",
    "Decimal",
    "Currency",
    "Percent",
    "Boolean",
];

fn number_kind(flags: u16) -> usize {
    if flags & num::AS_ENTERED != 0 {
        0
    } else if flags & num::GENERAL != 0 {
        1
    } else if flags & num::BOOLEAN != 0 {
        5
    } else if flags & num::NOTATION != 0 && flags & num::PERCENT != 0 {
        4
    } else if flags & num::NOTATION != 0 {
        3
    } else {
        2
    }
}

fn with_number_kind(flags: u16, k: usize) -> u16 {
    let f = flags
        & !(num::AS_ENTERED
            | num::GENERAL
            | num::BOOLEAN
            | num::NOTATION
            | num::PERCENT
            | num::CURRENCY);
    match k {
        0 => f | num::AS_ENTERED,
        1 => f | num::GENERAL,
        3 => f | num::NOTATION | num::CURRENCY,
        4 => f | num::NOTATION | num::PERCENT,
        5 => f | num::BOOLEAN,
        _ => f,
    }
}

const DATE_FORMATS: [u8; 10] = [0, 2, 7, 3, 4, 5, 6, 8, 9, 1];
const DATE_ELEMENTS: [&str; 11] = [
    "None",
    "Day",
    "Weekday (abbreviated)",
    "Weekday",
    "Month (number)",
    "Month (abbreviated)",
    "Month",
    "Year (2 digits)",
    "Year (4 digits)",
    "Quarter (short)",
    "Quarter",
];
const PADS: [&str; 3] = ["None", "Leading zero", "Leading space"];

fn char_edit(ui: &mut egui::Ui, c: &mut char) -> bool {
    let mut s = if *c == '\0' {
        String::new()
    } else {
        c.to_string()
    };
    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(24.0));
    if r.changed() {
        *c = s.chars().next().unwrap_or('\0');
        return true;
    }
    false
}

impl Designer {
    /// Inserts `o` as the last child of `parent` (after the container and
    /// its other children in the object list, as FileMaker stores them).
    fn add_child(&mut self, parent: u64, mut o: LayoutObject) -> u64 {
        o.id = self.alloc_id();
        o.parent = Some(parent);
        if o.options == 0 {
            o.options = ly::anchor::LEFT | ly::anchor::TOP;
        }
        let at = self
            .layout
            .objects
            .iter()
            .rposition(|x| x.id == parent || x.parent == Some(parent))
            .map_or(self.layout.objects.len(), |p| p + 1);
        let id = o.id;
        self.layout.objects.insert(at, o);
        id
    }

    /// Insert › Popover Button: the button, and its popover under it.
    pub(super) fn insert_popover_button(&mut self, rect: Bounds) {
        self.checkpoint();
        let pop = Bounds {
            top: rect.bottom + 8.0,
            left: rect.left,
            bottom: rect.bottom + 208.0,
            right: rect.left + 290.0,
        };
        let p = self.add(LayoutObject {
            kind: kind::POPOVER,
            bounds: pop,
            style_raw: new_style(POPOVER_TYPE),
            label: Some(fmp12::save::text_literal_tokens(tr("Popover"))),
            ..Default::default()
        });
        let b = self.add(LayoutObject {
            kind: kind::POPOVER_BUTTON,
            bounds: rect,
            options: ly::anchor::LEFT | ly::anchor::TOP | NEW_BUTTON_BIT,
            style_raw: new_style(POPOVER_BUTTON_TYPE),
            text: Some(tr("Popover").to_string()),
            popover: Some(p),
            ..Default::default()
        });
        if let Some(x) = self.object_mut(p) {
            x.popover = Some(b);
        }
    }

    /// Insert › Slide Control: three panels.
    pub(super) fn insert_slide_control(&mut self, rect: Bounds) {
        self.checkpoint();
        let sc = self.add(LayoutObject {
            kind: kind::SLIDE_CONTROL,
            bounds: rect.clone(),
            style_raw: new_style(SLIDE_TYPE),
            ..Default::default()
        });
        let inner = Bounds {
            top: 0.0,
            left: 0.0,
            bottom: rect.bottom - rect.top,
            right: rect.right - rect.left,
        };
        let mut first = None;
        for _ in 0..3 {
            let id = self.add_child(
                sc,
                LayoutObject {
                    kind: kind::SLIDE_PANEL,
                    bounds: inner.clone(),
                    style_raw: new_style(SLIDE_PANEL_TYPE),
                    ..Default::default()
                },
            );
            first.get_or_insert(id);
        }
        self.place_inside(sc);
        self.sel = [sc].into();
    }

    /// Insert › Button Bar: three segments without actions (Button Setup
    /// gives them one).
    pub(super) fn insert_button_bar(&mut self, rect: Bounds) {
        self.checkpoint();
        let bar = self.add(LayoutObject {
            kind: kind::BUTTON_BAR,
            bounds: rect.clone(),
            style_raw: new_style(BAR_TYPE),
            ..Default::default()
        });
        let w = (rect.right - rect.left) / 3.0;
        for i in 0..3 {
            let label = format!("{} {}", tr("Button"), i + 1);
            self.add_child(
                bar,
                LayoutObject {
                    kind: kind::BUTTON,
                    bounds: Bounds {
                        top: 0.0,
                        left: w * i as f64,
                        bottom: rect.bottom - rect.top,
                        right: w * (i + 1) as f64,
                    },
                    options: ly::anchor::LEFT | ly::anchor::TOP | NEW_BUTTON_BIT,
                    style_raw: new_style(SEGMENT_TYPE),
                    label: Some(fmp12::save::text_literal_tokens(&label)),
                    text: Some(label),
                    ..Default::default()
                },
            );
        }
        self.place_inside(bar);
        self.sel = [bar].into();
    }

    /// Web Viewer Setup, opened by the tool or a double-click.
    pub(super) fn open_web_setup(&mut self, rect: Bounds, edit: Option<u64>, doc: &Doc) {
        let url = edit
            .and_then(|id| self.object(id))
            .and_then(|o| o.url.as_ref())
            .map(|t| calc_text(doc, t))
            .unwrap_or_else(|| "\"https://\"".into());
        self.extra.web = Some((rect, edit, url));
    }

    /// Web Viewer Setup for a new web viewer.
    pub(super) fn new_web_setup(&mut self, rect: Bounds) {
        self.extra.web = Some((rect, None, "\"https://\"".into()));
    }

    fn insert_web_viewer(&mut self, rect: Bounds, url: Vec<u8>) {
        self.checkpoint();
        let id = self.add(LayoutObject {
            kind: kind::EXTERNAL,
            bounds: rect,
            style_raw: new_style(WEB_TYPE),
            external: Some((ly::WEB_VIEWER, WEB_FLAGS)),
            url: Some(url),
            ..Default::default()
        });
        self.place_inside(id);
    }

    /// Chart Setup for a chart object (double-click, Format menu, Inspector).
    pub(super) fn open_chart_setup(&mut self, id: u64, doc: &Doc) {
        if let Some(c) = self.object(id).and_then(|o| o.chart.as_deref()) {
            self.extra.chart = Some(ChartEdit::read(doc, id, c));
        }
    }

    /// The segments of button bar `bar`, left to right.
    fn segments(&self, bar: u64) -> Vec<u64> {
        let mut s: Vec<&LayoutObject> = self
            .layout
            .objects
            .iter()
            .filter(|o| o.parent == Some(bar))
            .collect();
        s.sort_by(|a, b| a.bounds.left.total_cmp(&b.bounds.left));
        s.iter().map(|o| o.id).collect()
    }

    /// Segments share the bar's width equally, as FileMaker lays them out.
    fn relayout_bar(&mut self, bar: u64) {
        let Some(b) = self.object(bar).map(|o| o.bounds.clone()) else {
            return;
        };
        let segs = self.segments(bar);
        let w = (b.right - b.left) / segs.len().max(1) as f64;
        for (i, id) in segs.into_iter().enumerate() {
            if let Some(o) = self.object_mut(id) {
                o.bounds = Bounds {
                    top: 0.0,
                    left: (w * i as f64).round(),
                    bottom: b.bottom - b.top,
                    right: (w * (i + 1) as f64).round(),
                };
            }
        }
    }

    /// Button bar › Add segment (a label, no action yet).
    pub(super) fn add_segment(&mut self, bar: u64) {
        self.checkpoint();
        let label = format!("{} {}", tr("Button"), self.segments(bar).len() + 1);
        self.add_child(
            bar,
            LayoutObject {
                kind: kind::BUTTON,
                options: ly::anchor::LEFT | ly::anchor::TOP | NEW_BUTTON_BIT,
                style_raw: new_style(SEGMENT_TYPE),
                label: Some(fmp12::save::text_literal_tokens(&label)),
                text: Some(label),
                ..Default::default()
            },
        );
        self.relayout_bar(bar);
    }

    /// Button bar › Delete segment (a bar keeps at least one); the active
    /// segment moves to the first one if it was this one.
    pub(super) fn delete_segment(&mut self, bar: u64, seg: u64) {
        if self.segments(bar).len() < 2 {
            return;
        }
        self.checkpoint();
        self.layout.objects.retain(|o| o.id != seg);
        self.layout.tab_order.retain(|id| *id != seg);
        let first = self.segments(bar).first().copied();
        if let Some(b) = self.object_mut(bar)
            && b.selected == Some(seg)
        {
            b.selected = first;
        }
        self.relayout_bar(bar);
    }

    /// Inspector › Data of web viewers, button bars and charts; `false` for
    /// other objects.
    pub(super) fn object_data(&mut self, ui: &mut egui::Ui, doc: &Doc, o: &LayoutObject) -> bool {
        match o.kind {
            kind::EXTERNAL if o.external.is_some_and(|e| e.0 == ly::WEB_VIEWER) => {
                ui.label(tr("Web address"));
                self.calc_field(ui, doc, o.id, "url", |o| o.url.as_ref(), |o, t| o.url = t);
                let flags = o.external.map_or(0, |e| e.1);
                for (bit, name) in WEB_OPTIONS {
                    let mut on = flags & bit != 0;
                    if ui.checkbox(&mut on, tr(name)).changed() {
                        self.touch("web_options");
                        if let Some(x) = self.object_mut(o.id)
                            && let Some(e) = x.external.as_mut()
                        {
                            e.1 ^= bit;
                        }
                    }
                }
                if ui.button(tr("Web Viewer Setup…")).clicked() {
                    self.open_web_setup(o.bounds.clone(), Some(o.id), doc);
                }
            }
            kind::EXTERNAL if o.chart.is_some() => {
                if ui.button(tr("Chart Setup…")).clicked() {
                    self.open_chart_setup(o.id, doc);
                }
            }
            kind::BUTTON_BAR => {
                let segs = self.segments(o.id);
                let to = (self.layout.table_occurrence & 0xFFF) as u32;
                let table = doc.db.table_of(to).map(|t| t.id).unwrap_or(0);
                let name = |s: &LayoutObject| match &s.label {
                    Some(t) => doc.db.eval_tokens(t, table, to, None).to_text(),
                    None => s.text.clone().unwrap_or_default(),
                };
                let names: Vec<(u64, String)> = segs
                    .iter()
                    .filter_map(|id| self.object(*id))
                    .map(|s| (s.id, name(s)))
                    .collect();
                ui.label(tr("Active segment"));
                let mut sel = o.selected;
                let shown = |v: Option<u64>| {
                    names
                        .iter()
                        .find(|n| Some(n.0) == v)
                        .map_or(tr("<None>").to_string(), |n| n.1.clone())
                };
                egui::ComboBox::from_id_salt("bar_active")
                    .selected_text(shown(sel))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sel, None, tr("<None>"));
                        for (id, n) in &names {
                            ui.selectable_value(&mut sel, Some(*id), n);
                        }
                    });
                if sel != o.selected {
                    self.touch("bar_active");
                    if let Some(x) = self.object_mut(o.id) {
                        x.selected = sel;
                    }
                }
                ui.separator();
                for (i, (id, _)) in names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} {}", tr("Segment"), i + 1));
                        if ui.button(tr("Action…")).clicked() {
                            self.open_button_setup(*id, doc);
                        }
                        if ui
                            .add_enabled(names.len() > 1, egui::Button::new(tr("Delete")))
                            .clicked()
                        {
                            self.delete_segment(o.id, *id);
                        }
                    });
                    self.calc_field(
                        ui,
                        doc,
                        *id,
                        "label",
                        |o| o.label.as_ref(),
                        |o, t| o.label = t,
                    );
                }
                if ui.button(tr("Add segment")).clicked() {
                    self.add_segment(o.id);
                }
            }
            _ => return false,
        }
        ui.separator();
        true
    }

    fn chart_dialog(&mut self, ctx: &egui::Context, doc: &Doc) {
        let Some(mut e) = self.extra.chart.take() else {
            return;
        };
        let mut done = None;
        let layout_to = (self.layout.table_occurrence & 0xFFF) as u32;
        let related = super::related(doc, layout_to);
        egui::Window::new(tr("Chart Setup"))
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                let bubble = e.chart.kind == Kind::Bubble;
                egui::Grid::new("chart_setup").num_columns(2).show(ui, |ui| {
                    ui.label(tr("Chart type"));
                    let kind_name =
                        |k: Kind| CHART_KINDS.iter().find(|x| x.0 == k).map_or("", |x| tr(x.1));
                    egui::ComboBox::from_id_salt("chart_kind")
                        .selected_text(kind_name(e.chart.kind))
                        .show_ui(ui, |ui| {
                            // A bubble chart's size series has no other use.
                            for (k, n) in CHART_KINDS {
                                if (k == Kind::Bubble) == bubble {
                                    ui.selectable_value(&mut e.chart.kind, k, tr(n));
                                }
                            }
                        });
                    ui.end_row();
                    for (label, s) in [
                        ("Title", &mut e.title),
                        ("X-axis title", &mut e.x_title),
                        ("Y-axis title", &mut e.y_title),
                        ("X-axis data", &mut e.x),
                    ] {
                        ui.label(tr(label));
                        ui.add(egui::TextEdit::singleline(s).desired_width(300.0));
                        ui.end_row();
                    }
                    for (i, (v, n)) in e.series.iter_mut().enumerate() {
                        ui.label(format!("{} {}", tr("Series"), i + 1));
                        ui.add(egui::TextEdit::singleline(v).desired_width(300.0));
                        ui.end_row();
                        ui.label(tr("Series name"));
                        ui.add(egui::TextEdit::singleline(n).desired_width(300.0));
                        ui.end_row();
                    }
                    if bubble {
                        ui.label(tr("Bubble size"));
                        ui.add(egui::TextEdit::singleline(&mut e.size).desired_width(300.0));
                        ui.end_row();
                    }
                    ui.label(tr("Data source"));
                    let mut src = match e.chart.source {
                        Source::FoundSet => 0,
                        Source::CurrentRecord => 1,
                        Source::Related(_) => 2,
                    };
                    ui.vertical(|ui| {
                        ui.radio_value(&mut src, 0, tr("Current Found Set"));
                        ui.radio_value(&mut src, 1, tr("Current Record (delimited data)"));
                        ui.add_enabled_ui(!related.is_empty(), |ui| {
                            ui.radio_value(&mut src, 2, tr("Related Records"));
                        });
                        let mut to = match e.chart.source {
                            Source::Related(t) => t,
                            _ => related.first().map_or(0, |r| r.0),
                        };
                        if src == 2 {
                            egui::ComboBox::from_id_salt("chart_to")
                                .selected_text(
                                    related.iter().find(|r| r.0 == to).map_or("", |r| r.1.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for (id, n) in &related {
                                        ui.selectable_value(&mut to, *id, n);
                                    }
                                });
                        }
                        e.chart.source = match src {
                            0 => Source::FoundSet,
                            1 => Source::CurrentRecord,
                            _ => Source::Related(to),
                        };
                    });
                    ui.end_row();
                });
                ui.checkbox(&mut e.chart.legend, tr("Show legend"));
                ui.checkbox(&mut e.chart.data_points, tr("Show data points on chart"));
                if let Some(err) = &e.error {
                    ui.colored_label(Color32::from_rgb(190, 40, 40), err);
                }
                ui.weak(tr(
                    "Series can be edited, not added or removed (the chart's per-series settings are not decoded).",
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr("OK")).clicked() {
                        done = Some(true);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => match e.build(doc) {
                Ok(c) => self.set_chart(e.id, c),
                Err(err) => {
                    e.error = Some(err);
                    self.extra.chart = Some(e);
                }
            },
            Some(false) => {}
            None => self.extra.chart = Some(e),
        }
    }

    /// Replaces the chart settings of object `id` (one undo step).
    fn set_chart(&mut self, id: u64, c: Chart) {
        if self.object(id).and_then(|o| o.chart.as_deref()) == Some(&c) {
            return;
        }
        self.checkpoint();
        if let Some(o) = self.object_mut(id) {
            o.chart = Some(Box::new(c));
        }
    }
    /// Format menu (Layout mode).
    pub fn format_menu(&mut self, ui: &mut egui::Ui, doc: &Doc) {
        let one = self.sel.len() == 1;
        if ui
            .add_enabled(one, egui::Button::new(tr("Conditional Formatting…")))
            .clicked()
            && let Some(o) = self.sel.first().and_then(|id| self.object(*id)).cloned()
        {
            let rows = o.conditions.iter().map(|k| CondRow::read(doc, k)).collect();
            self.extra.cond = Some((o.id, rows, 0));
            ui.close();
        }
        let web = self
            .sel
            .first()
            .and_then(|id| self.object(*id))
            .filter(|o| o.external.is_some_and(|e| e.0 == ly::WEB_VIEWER) && one)
            .cloned();
        if ui
            .add_enabled(web.is_some(), egui::Button::new(tr("Web Viewer Setup…")))
            .clicked()
            && let Some(o) = web
        {
            self.open_web_setup(o.bounds.clone(), Some(o.id), doc);
            ui.close();
        }
        let chart = self
            .sel
            .first()
            .filter(|id| one && self.object(**id).is_some_and(|o| o.chart.is_some()))
            .copied();
        if ui
            .add_enabled(chart.is_some(), egui::Button::new(tr("Chart Setup…")))
            .clicked()
            && let Some(id) = chart
        {
            self.open_chart_setup(id, doc);
            ui.close();
        }
    }

    /// Arrange › Set Tab Order…
    pub(super) fn tab_order_item(&mut self, ui: &mut egui::Ui) {
        if ui.button(tr("Set Tab Order…")).clicked() {
            let mut order = self.layout.tab_order.clone();
            order.retain(|id| self.object(*id).is_some());
            self.extra.tabs = Some((order, 0));
            ui.close();
        }
    }

    /// A calculation of the object, edited as text and compiled as it is
    /// typed (`slot` names it; `get`/`set` read and write the tokens).
    fn calc_field(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Doc,
        id: u64,
        slot: &'static str,
        get: fn(&LayoutObject) -> Option<&Vec<u8>>,
        set: fn(&mut LayoutObject, Option<Vec<u8>>),
    ) {
        let Some(o) = self.object(id) else { return };
        let cur = get(o).map(|t| calc_text(doc, t)).unwrap_or_default();
        let buf = self.extra.calcs.entry((id, slot)).or_insert(cur);
        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
        let text = buf.clone();
        let result = if text.trim().is_empty() {
            Ok(None)
        } else {
            compile(doc, &text).map(Some)
        };
        if let Err(e) = &result {
            ui.colored_label(Color32::from_rgb(190, 40, 40), e);
        }
        if r.changed()
            && let Ok(tokens) = result
        {
            self.touch(slot);
            if let Some(o) = self.object_mut(id) {
                set(o, tokens);
            }
        }
    }

    /// Inspector › Position additions: tooltip and the sliding options.
    pub(super) fn position_extra(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        if let [id] = ids {
            ui.label(tr("Tooltip"));
            self.calc_field(
                ui,
                doc,
                *id,
                "tooltip",
                |o| o.tooltip.as_ref(),
                |o, t| o.tooltip = t,
            );
        }
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        for (bit, name) in [
            (ly::SLIDE_DIRECTLY_ABOVE, "Only objects directly above"),
            (ly::RESIZE_PART, "Also resize enclosing part"),
        ] {
            let mut on = o.options & bit != 0;
            if ui
                .add_enabled(
                    o.options & ly::SLIDE_UP != 0,
                    egui::Checkbox::new(&mut on, tr(name)),
                )
                .changed()
            {
                self.touch(name);
                for x in self
                    .layout
                    .objects
                    .iter_mut()
                    .filter(|x| ids.contains(&x.id))
                {
                    x.options = if on {
                        x.options | bit
                    } else {
                        x.options & !bit
                    };
                }
            }
        }
    }

    /// Inspector › Data additions: placeholder, "hide object when", and the
    /// data formats of a field.
    pub(super) fn data_extra(&mut self, ui: &mut egui::Ui, doc: &Doc, ids: &[u64]) {
        let Some(o) = self.object(ids[0]).cloned() else {
            return;
        };
        if o.kind == kind::FIELD && ids.len() == 1 {
            ui.label(tr("Placeholder text"));
            self.calc_field(
                ui,
                doc,
                o.id,
                "placeholder",
                |o| o.placeholder.as_ref(),
                |o, t| o.placeholder = t,
            );
        }
        if ids.len() == 1 {
            ui.label(tr("Hide object when"));
            self.calc_field(
                ui,
                doc,
                o.id,
                "hide",
                |o| o.hide.as_ref(),
                |o, t| o.hide = t,
            );
            let mut find = o.options & ly::HIDE_IN_FIND != 0;
            if ui.checkbox(&mut find, tr("Apply in Find mode")).changed() {
                self.touch("hide_find");
                if let Some(x) = self.object_mut(o.id) {
                    x.options ^= ly::HIDE_IN_FIND;
                }
            }
        }
        if o.kind != kind::FIELD {
            return;
        }
        ui.separator();
        ui.label(tr("Data formatting"));
        let Some(mut f) = o.format.clone() else {
            if ui.button(tr("Set data formats")).clicked() {
                self.touch("format");
                for x in self
                    .layout
                    .objects
                    .iter_mut()
                    .filter(|x| ids.contains(&x.id))
                {
                    x.format = Some(Format::new_default());
                }
            }
            return;
        };
        let before = f.clone();
        format_editor(ui, doc, &mut f);
        if f != before {
            self.touch("format");
            for x in self
                .layout
                .objects
                .iter_mut()
                .filter(|x| ids.contains(&x.id) && x.kind == kind::FIELD)
            {
                x.format = Some(f.clone());
            }
        }
    }

    /// Conditional Formatting, Set Tab Order and Web Viewer Setup dialogs.
    pub(super) fn object_dialogs(&mut self, ctx: &egui::Context, doc: &Doc) {
        self.conditions_dialog(ctx, doc);
        self.tab_order_dialog(ctx);
        self.web_dialog(ctx, doc);
        self.chart_dialog(ctx, doc);
    }

    fn conditions_dialog(&mut self, ctx: &egui::Context, doc: &Doc) {
        let Some((id, mut rows, mut at)) = self.extra.cond.take() else {
            return;
        };
        let mut done = None;
        let mut error = None;
        egui::Window::new(tr("Conditional Formatting"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                egui::Grid::new("cond_list").striped(true).show(ui, |ui| {
                    for (i, r) in rows.iter().enumerate() {
                        let text = match r.op {
                            op::FORMULA => format!("{} {}", tr("Formula is"), r.formula),
                            _ => {
                                let name = OPS.iter().find(|x| x.0 == r.op).map_or("", |x| tr(x.1));
                                format!("{name} {} {}", r.value1, r.value2)
                            }
                        };
                        if ui.selectable_label(i == at, text).clicked() {
                            at = i;
                        }
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(tr("Add")).clicked() {
                        rows.push(CondRow::new());
                        at = rows.len() - 1;
                    }
                    if ui
                        .add_enabled(!rows.is_empty(), egui::Button::new(tr("Delete")))
                        .clicked()
                    {
                        rows.remove(at.min(rows.len() - 1));
                        at = at.saturating_sub(1);
                    }
                });
                if let Some(r) = rows.get_mut(at) {
                    ui.separator();
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("cond_op")
                            .selected_text(OPS.iter().find(|x| x.0 == r.op).map_or("", |x| tr(x.1)))
                            .show_ui(ui, |ui| {
                                for (v, name) in OPS {
                                    ui.selectable_value(&mut r.op, v, tr(name));
                                }
                            });
                        if r.op == op::FORMULA {
                            ui.add(egui::TextEdit::singleline(&mut r.formula).desired_width(280.0));
                        } else if !matches!(
                            r.op,
                            op::EMPTY | op::BEFORE_TODAY | op::TODAY | op::AFTER_TODAY
                        ) {
                            ui.add(egui::TextEdit::singleline(&mut r.value1).desired_width(120.0));
                            if matches!(r.op, op::BETWEEN | op::NOT_BETWEEN) {
                                ui.label(tr("and"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut r.value2).desired_width(120.0),
                                );
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.bold, tr("Bold"));
                        ui.checkbox(&mut r.italic, tr("Italic"));
                        ui.checkbox(&mut r.underline, tr("Underline"));
                    });
                    ui.horizontal(|ui| {
                        let mut on = r.color.is_some();
                        ui.checkbox(&mut on, tr("Text color"));
                        let mut c = r.color.unwrap_or(Color32::RED);
                        ui.color_edit_button_srgba(&mut c);
                        r.color = on.then_some(c);
                        let mut on = r.fill.is_some();
                        ui.checkbox(&mut on, tr("Fill color"));
                        let mut c = r.fill.unwrap_or(Color32::YELLOW);
                        ui.color_edit_button_srgba(&mut c);
                        r.fill = on.then_some(c);
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr("OK")).clicked() {
                        done = Some(true);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                let built: Result<Vec<Condition>, String> =
                    rows.iter().map(|r| r.build(doc)).collect();
                match built {
                    Ok(conds) => {
                        self.checkpoint();
                        if let Some(o) = self.object_mut(id) {
                            o.conditions = conds;
                        }
                    }
                    Err(e) => {
                        error = Some(e);
                        self.extra.cond = Some((id, rows, at));
                    }
                }
            }
            Some(false) => {}
            None => self.extra.cond = Some((id, rows, at)),
        }
        if let Some(e) = error {
            ctx.data_mut(|d| d.insert_temp(egui::Id::NULL.with("cond_error"), e));
        }
    }

    fn tab_order_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut order, mut at)) = self.extra.tabs.take() else {
            return;
        };
        let mut done = None;
        let name = |o: &LayoutObject| -> String {
            let what = match o.kind {
                kind::FIELD => tr("Field"),
                kind::BUTTON => tr("Button"),
                kind::BUTTON_BAR => tr("Button bar"),
                kind::POPOVER_BUTTON => tr("Popover button"),
                kind::TAB_CONTROL => tr("Tab control"),
                kind::SLIDE_CONTROL => tr("Slide control"),
                kind::EXTERNAL => tr("Web viewer"),
                _ => tr("Object"),
            };
            let n = if o.name.is_empty() {
                o.text.clone().unwrap_or_default()
            } else {
                o.name.clone()
            };
            format!("{what} {} {n}", o.id)
        };
        let candidates: Vec<(u64, String)> = self
            .layout
            .objects
            .iter()
            .filter(|o| {
                matches!(
                    o.kind,
                    kind::FIELD
                        | kind::BUTTON
                        | kind::BUTTON_BAR
                        | kind::POPOVER_BUTTON
                        | kind::TAB_CONTROL
                        | kind::SLIDE_CONTROL
                        | kind::EXTERNAL
                ) && !order.contains(&o.id)
            })
            .map(|o| (o.id, name(o)))
            .collect();
        let labels: BTreeMap<u64, String> = self
            .layout
            .objects
            .iter()
            .map(|o| (o.id, name(o)))
            .collect();
        egui::Window::new(tr("Set Tab Order"))
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(tr("Objects in the tab order, first to last:"));
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (i, id) in order.iter().enumerate() {
                            let t = format!(
                                "{}. {}",
                                i + 1,
                                labels.get(id).cloned().unwrap_or_default()
                            );
                            if ui.selectable_label(i == at, t).clicked() {
                                at = i;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    let n = order.len();
                    if ui
                        .add_enabled(at > 0 && at < n, egui::Button::new("▲"))
                        .clicked()
                    {
                        order.swap(at, at - 1);
                        at -= 1;
                    }
                    if ui.add_enabled(at + 1 < n, egui::Button::new("▼")).clicked() {
                        order.swap(at, at + 1);
                        at += 1;
                    }
                    if ui
                        .add_enabled(at < n, egui::Button::new(tr("Remove")))
                        .clicked()
                    {
                        order.remove(at);
                        at = at.min(order.len().saturating_sub(1));
                    }
                });
                ui.menu_button(tr("Add…"), |ui| {
                    for (id, n) in &candidates {
                        if ui.button(n).clicked() {
                            order.push(*id);
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr("OK")).clicked() {
                        done = Some(true);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                if order != self.layout.tab_order {
                    self.checkpoint();
                    self.layout.tab_order = order;
                }
            }
            Some(false) => {}
            None => self.extra.tabs = Some((order, at)),
        }
    }

    fn web_dialog(&mut self, ctx: &egui::Context, doc: &Doc) {
        let Some((rect, edit, mut url)) = self.extra.web.take() else {
            return;
        };
        let mut done = None;
        let compiled = compile(doc, &url);
        egui::Window::new(tr("Web Viewer Setup"))
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(tr("Web address (a calculation):"));
                ui.add(
                    egui::TextEdit::multiline(&mut url)
                        .desired_rows(3)
                        .desired_width(360.0),
                );
                if let Err(e) = &compiled {
                    ui.colored_label(Color32::from_rgb(190, 40, 40), e);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(compiled.is_ok(), egui::Button::new(tr("OK")))
                        .clicked()
                    {
                        done = Some(true);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        done = Some(false);
                    }
                });
            });
        match (done, compiled) {
            (Some(true), Ok(tokens)) => match edit {
                Some(id) => {
                    self.checkpoint();
                    if let Some(o) = self.object_mut(id) {
                        o.url = Some(tokens);
                    }
                }
                None => self.insert_web_viewer(rect, tokens),
            },
            (Some(_), _) => {}
            (None, _) => self.extra.web = Some((rect, edit, url)),
        }
    }
}

/// Number, date, time and graphic formats of a field object.
fn format_editor(ui: &mut egui::Ui, doc: &Doc, f: &mut Format) {
    let loc = &doc.db.locale;
    egui::CollapsingHeader::new(tr("Number"))
        .default_open(true)
        .show(ui, |ui| {
            let n = &mut f.number;
            let mut k = number_kind(n.flags);
            egui::ComboBox::from_id_salt("num_kind")
                .selected_text(tr(NUMBER_FORMATS[k]))
                .show_ui(ui, |ui| {
                    for (i, name) in NUMBER_FORMATS.iter().enumerate() {
                        ui.selectable_value(&mut k, i, tr(name));
                    }
                });
            n.flags = with_number_kind(n.flags, k);
            let bit = |ui: &mut egui::Ui, flags: &mut u16, b: u16, name: &'static str| {
                let mut on = *flags & b != 0;
                if ui.checkbox(&mut on, tr(name)).changed() {
                    *flags ^= b;
                }
            };
            match k {
                2..=4 => {
                    ui.horizontal(|ui| {
                        bit(
                            ui,
                            &mut n.flags,
                            num::FIXED_DIGITS,
                            "Fixed number of decimal digits",
                        );
                        ui.add(egui::DragValue::new(&mut n.digits).range(0..=15));
                    });
                    bit(ui, &mut n.flags, num::THOUSANDS, "Use thousands separator");
                    bit(
                        ui,
                        &mut n.flags,
                        num::SYSTEM_SEPARATORS,
                        "Use system separators",
                    );
                    ui.horizontal(|ui| {
                        ui.label(tr("Decimal"));
                        char_edit(ui, &mut n.decimal);
                        ui.label(tr("Thousands"));
                        char_edit(ui, &mut n.thousands);
                    });
                    if k == 3 {
                        ui.horizontal(|ui| {
                            ui.label(tr("Currency symbol"));
                            ui.add(egui::TextEdit::singleline(&mut n.currency).desired_width(40.0));
                            bit(ui, &mut n.flags, num::LEADING, "Leading");
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("Negative"));
                        let styles = [
                            "-1234",
                            "1234-",
                            "(1234)",
                            "<1234>",
                            "1234 CR",
                            "\u{25B2}1234",
                        ];
                        let mut s = n.negative_style.min(5) as usize;
                        egui::ComboBox::from_id_salt("num_neg")
                            .selected_text(styles[s])
                            .show_ui(ui, |ui| {
                                for (i, x) in styles.iter().enumerate() {
                                    ui.selectable_value(&mut s, i, *x);
                                }
                            });
                        n.negative_style = s as u8;
                    });
                    bit(ui, &mut n.flags, num::NEGATIVE_COLOR, "Negative in red");
                    bit(ui, &mut n.flags, num::HIDE_ZERO, "Hide zero");
                }
                5 => {
                    ui.horizontal(|ui| {
                        ui.label(tr("Non-zero"));
                        ui.add(egui::TextEdit::singleline(&mut n.true_text).desired_width(60.0));
                        ui.label(tr("Zero"));
                        ui.add(egui::TextEdit::singleline(&mut n.false_text).desired_width(60.0));
                    });
                }
                _ => {}
            }
        });
    ui.collapsing(tr("Date"), |ui| {
        let sample = calc::Value::Date(calc::value::day_number(2026, 3, 7));
        let show = |f: &Format| {
            print::format::apply(f, &sample, loc)
                .map_or_else(|| tr("As entered").to_string(), |x| x.0)
        };
        let mut fmt = f.date.format;
        egui::ComboBox::from_id_salt("date_fmt")
            .selected_text(if fmt == 1 {
                tr("Custom").to_string()
            } else {
                show(f)
            })
            .show_ui(ui, |ui| {
                for v in DATE_FORMATS {
                    let mut g = f.clone();
                    g.date.format = v;
                    let name = if v == 1 {
                        tr("Custom").to_string()
                    } else {
                        show(&g)
                    };
                    ui.selectable_value(&mut fmt, v, name);
                }
            });
        f.date.format = fmt;
        let d = &mut f.date;
        if matches!(fmt, 1 | 2 | 7) {
            for (label, v, salt) in [
                ("Day", &mut d.day_style, "ds"),
                ("Month", &mut d.month_style, "ms"),
            ] {
                ui.horizontal(|ui| {
                    ui.label(tr(label));
                    let mut s = (*v).min(2) as usize;
                    egui::ComboBox::from_id_salt(salt)
                        .selected_text(tr(PADS[s]))
                        .show_ui(ui, |ui| {
                            for (i, p) in PADS.iter().enumerate() {
                                ui.selectable_value(&mut s, i, tr(p));
                            }
                        });
                    *v = s as u8;
                });
            }
        }
        if matches!(fmt, 2 | 7) {
            ui.horizontal(|ui| {
                ui.label(tr("Separator"));
                char_edit(ui, &mut d.separator);
            });
        }
        if fmt == 1 {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut d.separators[0]).desired_width(24.0));
                for k in 0..4 {
                    let mut e = d.elements[k].min(10) as usize;
                    egui::ComboBox::from_id_salt(("date_el", k))
                        .width(90.0)
                        .selected_text(tr(DATE_ELEMENTS[e]))
                        .show_ui(ui, |ui| {
                            for (i, n) in DATE_ELEMENTS.iter().enumerate() {
                                ui.selectable_value(&mut e, i, tr(n));
                            }
                        });
                    d.elements[k] = e as u8;
                    ui.add(
                        egui::TextEdit::singleline(&mut d.separators[k + 1]).desired_width(24.0),
                    );
                }
            });
            ui.weak(show(f));
        }
    });
    ui.collapsing(tr("Time"), |ui| {
        let t = &mut f.time;
        let mut custom = t.flags & time::FORMATTED != 0 && t.flags & time::H24 == 0;
        ui.horizontal(|ui| {
            ui.radio_value(&mut custom, false, tr("As entered"));
            ui.radio_value(&mut custom, true, tr("Custom"));
        });
        if !custom {
            t.flags = 0x8F;
            return;
        }
        let mut f2 = t.flags & !(time::H24) | time::FORMATTED;
        ui.horizontal(|ui| {
            for (b, n) in [
                (time::HOURS, "Hours"),
                (time::MINUTES, "Minutes"),
                (time::SECONDS, "Seconds"),
            ] {
                let mut on = f2 & b != 0;
                if ui.checkbox(&mut on, tr(n)).changed() {
                    f2 ^= b;
                }
            }
        });
        if f2 & (time::HOURS | time::MINUTES | time::SECONDS) == 0 {
            f2 |= time::HOURS | time::MINUTES;
        }
        t.flags = f2;
        ui.horizontal(|ui| {
            ui.label(tr("Separator"));
            char_edit(ui, &mut t.separator);
            ui.label("AM");
            ui.add(egui::TextEdit::singleline(&mut t.am).desired_width(40.0));
            ui.label("PM");
            ui.add(egui::TextEdit::singleline(&mut t.pm).desired_width(40.0));
        });
        for (label, v, salt) in [
            ("Hours", &mut t.hour_style, "hs"),
            ("Minutes and seconds", &mut t.minsec_style, "mss"),
        ] {
            ui.horizontal(|ui| {
                ui.label(tr(label));
                let mut s = (*v).min(2) as usize;
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(tr(PADS[s]))
                    .show_ui(ui, |ui| {
                        for (i, p) in PADS.iter().enumerate() {
                            ui.selectable_value(&mut s, i, tr(p));
                        }
                    });
                *v = s as u8;
            });
        }
    });
    if let Some(g) = f.graphic.as_mut() {
        ui.horizontal(|ui| {
            // ponytail: the graphic format values are kept and edited as
            // FileMaker's numbers; their scaling meaning is not decoded.
            ui.label(tr("Graphic format"));
            ui.add(egui::DragValue::new(g).range(0..=7));
        });
    }
}

/// Layout-mode drawing of the objects of this module; `false` for others.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    painter: &egui::Painter,
    doc: &Doc,
    l: &Layout,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    table: u32,
    to: u32,
) -> bool {
    let thin = Stroke::new(0.5, Color32::from_gray(150));
    let label = |o: &LayoutObject| match &o.label {
        Some(t) => doc.db.eval_tokens(t, table, to, None).to_text(),
        None => o.text.clone().unwrap_or_default(),
    };
    match o.kind {
        kind::POPOVER_BUTTON => {
            lk.paint_box(painter, r);
            painter.rect_stroke(r, 4.0, thin, StrokeKind::Inside);
            lk.paint_text(painter, r, &label(o));
        }
        kind::POPOVER => {
            lk.paint_box(painter, r);
            let dash = egui::Shape::dashed_line(
                &[
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ],
                Stroke::new(1.0, Color32::from_gray(120)),
                5.0,
                3.0,
            );
            painter.extend(dash);
            let title = label(o);
            if !title.is_empty() {
                let bar = Rect::from_min_size(r.min, Vec2::new(r.width(), 22.0));
                painter.rect_filled(bar, 0.0, Color32::from_gray(236));
                painter.text(
                    bar.center(),
                    egui::Align2::CENTER_CENTER,
                    title,
                    lk.font(),
                    lk.color,
                );
            }
        }
        kind::SLIDE_CONTROL => {
            lk.paint_box(painter, r);
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            let n = l.objects.iter().filter(|x| x.parent == Some(o.id)).count();
            if o.flags & ly::slide::NO_DOTS == 0 {
                let x0 = r.center().x - 7.0 * (n as f32 - 1.0);
                for i in 0..n {
                    let c = egui::pos2(x0 + 14.0 * i as f32, r.bottom() - 10.0);
                    painter.circle_filled(
                        c,
                        4.0,
                        Color32::from_gray(if i == 0 { 60 } else { 190 }),
                    );
                }
            }
        }
        kind::BUTTON_BAR => {
            lk.paint_box(painter, r);
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
        }
        kind::EXTERNAL if o.external.is_some_and(|e| e.0 == ly::WEB_VIEWER) => {
            painter.rect_filled(r, 0.0, Color32::from_gray(246));
            painter.rect_stroke(r, 0.0, thin, StrokeKind::Inside);
            let url = o
                .url
                .as_ref()
                .map(|t| calc_text(doc, t))
                .unwrap_or_default();
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}\n{url}", tr("Web Viewer")),
                egui::FontId::proportional(11.0),
                Color32::from_gray(90),
            );
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::design::{LayoutOp, run_layout_op};

    #[test]
    fn number_kinds_round_trip() {
        for k in 0..6 {
            assert_eq!(number_kind(with_number_kind(0x0A0C, k)), k);
        }
        // Currency keeps the leading symbol and fixed digits.
        assert_eq!(with_number_kind(0x0900, 3), 0x0A04);
    }

    fn b(t: f64, l: f64, bt: f64, r: f64) -> Bounds {
        Bounds {
            top: t,
            left: l,
            bottom: bt,
            right: r,
        }
    }

    fn field_id(d: &Doc, table: &str, name: &str) -> u32 {
        d.db.tables
            .iter()
            .find(|t| t.name == table)
            .and_then(|t| t.field_by_name(name))
            .map(|f| f.id)
            .unwrap()
    }

    fn last(ds: &Designer) -> u64 {
        *ds.sel.iter().max().unwrap_or(&0)
    }

    /// "Objects Demo", a new layout of the Assets template made through the
    /// designer's commands: formatted fields, conditional formats, a
    /// placeholder, a hide condition, a popover, a slide control, a button
    /// bar running a script step, a web viewer and a tab order. Two records.
    pub(crate) fn objects_demo() -> Option<Doc> {
        let mut d = super::super::tests::template("Assets")?;
        let list = d.layouts.iter().position(|l| l.name == "Asset List")?;
        let to = (d.layouts[list].table_occurrence & 0xFFF) as u32;
        let new = LayoutOp::New {
            name: "Objects Demo".into(),
            to,
        };
        let i = run_layout_op(&mut d, list, &new).ok()?;
        let mut ds = Designer::new(&d, i);
        let bi = ds
            .layout
            .parts
            .iter()
            .position(|p| p.kind == ly::part::BODY)?;
        super::super::resize_part(&mut ds.layout, bi, 660.0);
        let y = ds.layout.parts[bi].top + 10.0;
        let ids: BTreeMap<&str, u32> = [
            "Name",
            "Price",
            "Purchase Date",
            "CreationTimestamp",
            "Description",
            "Serial Number",
            "Vendor",
        ]
        .into_iter()
        .map(|n| (n, field_id(&d, "Assets", n)))
        .collect();
        let f = |n: &str| ids[n];
        // Name: placeholder; red bold when "Laptop"; yellow when no price.
        ds.insert_field(&d, b(y, 140.0, y + 44.0, 340.0), to, f("Name"), true);
        let name = last(&ds);
        let mut laptop = CondRow::new();
        laptop.value1 = "Laptop".into();
        laptop.bold = true;
        let mut no_price = CondRow::new();
        no_price.op = op::FORMULA;
        no_price.formula = "IsEmpty ( Assets::Price )".into();
        no_price.color = None;
        no_price.fill = Some(Color32::from_rgb(255, 240, 150));
        let conds = vec![laptop.build(&d).ok()?, no_price.build(&d).ok()?];
        let placeholder = compile(&d, "\"Asset name\"").ok()?;
        let tip = compile(&d, "\"The asset's name\"").ok()?;
        if let Some(o) = ds.object_mut(name) {
            o.placeholder = Some(placeholder);
            o.tooltip = Some(tip);
            o.conditions = conds;
        }
        // Price: currency, thousands, parentheses, red when negative.
        ds.insert_field(
            &d,
            b(y + 50.0, 140.0, y + 94.0, 340.0),
            to,
            f("Price"),
            true,
        );
        let price = last(&ds);
        let mut fmt = Format::new_default();
        fmt.number.flags =
            with_number_kind(0x0800, 3) | num::FIXED_DIGITS | num::THOUSANDS | num::NEGATIVE_COLOR;
        fmt.number.currency = "$".into();
        fmt.number.digits = 2;
        fmt.number.negative_style = 2;
        ds.object_mut(price)?.format = Some(fmt);
        // Purchase Date: "Saturday, March 7, 2026".
        ds.insert_field(
            &d,
            b(y + 100.0, 140.0, y + 144.0, 400.0),
            to,
            f("Purchase Date"),
            true,
        );
        let date = last(&ds);
        let mut df = Format::new_default();
        df.date.format = 6;
        ds.object_mut(date)?.format = Some(df);
        // Creation timestamp: "Mar 7, 2026 2:05 PM".
        let r = b(y + 150.0, 140.0, y + 194.0, 400.0);
        ds.insert_field(&d, r, to, f("CreationTimestamp"), true);
        let stamp = last(&ds);
        let mut tf = Format::new_default();
        tf.date.format = 4;
        tf.time.flags = 0x83;
        tf.time.minsec_style = 1;
        ds.object_mut(stamp)?.format = Some(tf);
        // Description: hidden when the price is over 1,000; slides up.
        ds.insert_field(
            &d,
            b(y + 200.0, 140.0, y + 244.0, 400.0),
            to,
            f("Description"),
            true,
        );
        let desc = last(&ds);
        let hide = compile(&d, "Assets::Price > 1000").ok()?;
        if let Some(o) = ds.object_mut(desc) {
            o.hide = Some(hide);
            o.options |= ly::SLIDE_UP | ly::SLIDE_DIRECTLY_ABOVE | ly::RESIZE_PART;
        }
        // Popover button "Details" and, in its popover, the serial number.
        ds.insert_popover_button(b(y, 450.0, y + 44.0, 570.0));
        let pb = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::POPOVER_BUTTON)?
            .id;
        let pop = ds.object(pb)?.popover?;
        ds.object_mut(pb)?.text = Some("Details".into());
        if let Some(o) = ds.object_mut(pop) {
            o.label = Some(fmp12::save::text_literal_tokens("Asset details"));
            o.bounds = b(y + 54.0, 450.0, y + 194.0, 740.0);
        }
        ds.insert_field(
            &d,
            b(y + 100.0, 470.0, y + 144.0, 720.0),
            to,
            f("Serial Number"),
            false,
        );
        // Slide control: the vendor on the first panel, a text on the second.
        ds.insert_slide_control(b(y + 260.0, 20.0, y + 380.0, 330.0));
        ds.insert_field(
            &d,
            b(y + 280.0, 40.0, y + 324.0, 300.0),
            to,
            f("Vendor"),
            false,
        );
        let sc = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::SLIDE_CONTROL)?
            .id;
        let second = ds
            .layout
            .objects
            .iter()
            .filter(|o| o.parent == Some(sc))
            .map(|o| o.id)
            .nth(1)?;
        ds.add_child(
            second,
            LayoutObject {
                kind: kind::TEXT,
                bounds: b(20.0, 20.0, 44.0, 280.0),
                style_raw: new_style(70),
                text: Some("Second slide".into()),
                ..Default::default()
            },
        );
        // Button bar: the first segment goes to the Asset List layout.
        ds.insert_button_bar(b(y + 400.0, 20.0, y + 430.0, 380.0));
        let bar = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::BUTTON_BAR)?
            .id;
        let seg = ds
            .layout
            .objects
            .iter()
            .filter(|o| o.parent == Some(bar))
            .map(|o| o.id)
            .min()?;
        let list_id = d.layouts[list].id;
        let step = fmp12::scripts::targeted_step(fmp12::scripts::id::GO_TO_LAYOUT, list_id);
        if let Some(o) = ds.object_mut(seg) {
            o.button = Some(step);
            o.label = Some(fmp12::save::text_literal_tokens("Asset List"));
            o.text = Some("Asset List".into());
        }
        ds.object_mut(bar)?.selected = Some(seg);
        // Web viewer: an address computed from the name.
        let url = compile(&d, "\"https://example.com/?q=\" & Assets::Name").ok()?;
        ds.insert_web_viewer(b(y + 450.0, 20.0, y + 590.0, 520.0), url);
        // Tab order: name, price, date, the description, the timestamp.
        ds.layout.tab_order = vec![name, price, date, desc, stamp];
        ds.layout.width = ds.layout.width.max(760.0);
        ds.apply(&mut d);
        // Two records.
        let ti = d.db.tables.iter().position(|t| t.name == "Assets")?;
        let tid = d.db.tables[ti].id;
        let rows = [
            ("Laptop", "-1234.5", "3/7/2026"),
            ("Desk", "300", "1/15/2025"),
        ];
        for (n, p, day) in rows {
            let r = d.db.new_record(tid)?;
            let v = &mut d.db.tables[ti].records[r].values;
            v.insert(f("Name"), n.into());
            v.insert(f("Price"), p.into());
            v.insert(f("Purchase Date"), day.into());
            v.insert(f("Description"), format!("About the {n}"));
        }
        Some(d)
    }

    pub(crate) fn written(d: &Doc) -> Vec<u8> {
        let b = d.db.save(&d.file).unwrap();
        let f = fmp12::File::from_bytes(b).unwrap();
        fmp12::save::save_layouts(&f, &d.db.layouts).unwrap()
    }

    /// Everything the demo set survives writing and reading back.
    #[test]
    fn objects_round_trip() {
        let Some(d) = objects_demo() else { return };
        let f = fmp12::File::from_bytes(written(&d)).unwrap();
        let back = ly::layouts(&f).unwrap();
        let l = back.iter().find(|l| l.name == "Objects Demo").unwrap();
        let mine =
            d.db.layouts
                .iter()
                .find(|x| x.name == "Objects Demo")
                .unwrap();
        let count = |k| l.objects.iter().filter(|o| o.kind == k).count();
        assert_eq!(count(kind::POPOVER_BUTTON), 1);
        assert_eq!(count(kind::POPOVER), 1);
        assert_eq!(count(kind::SLIDE_CONTROL), 1);
        assert_eq!(count(kind::SLIDE_PANEL), 3);
        assert_eq!(count(kind::BUTTON_BAR), 1);
        let one = |k| l.objects.iter().find(|o| o.kind == k).unwrap();
        let (pb, pop) = (one(kind::POPOVER_BUTTON), one(kind::POPOVER));
        assert_eq!((pb.popover, pop.popover), (Some(pop.id), Some(pb.id)));
        let web = l.objects.iter().find(|o| o.url.is_some()).unwrap();
        assert_eq!(web.external, Some((ly::WEB_VIEWER, WEB_FLAGS)));
        for o in &mine.objects {
            let x = l.objects.iter().find(|x| x.id == o.id).unwrap();
            // New fields without formats take their prototype's.
            if o.format.is_some() {
                assert_eq!(x.format, o.format, "formats of {}", o.id);
            }
            assert_eq!(x.conditions, o.conditions, "conditions of {}", o.id);
            assert_eq!(x.hide, o.hide);
            assert_eq!(x.placeholder, o.placeholder);
            assert_eq!(x.tooltip, o.tooltip);
            assert_eq!(x.parent, o.parent, "parent of {}", o.id);
            assert_eq!(ly::effective_options(o), x.options, "options of {}", o.id);
        }
        assert_eq!(l.tab_order[..5], mine.tab_order[..]);
        assert!(one(kind::BUTTON_BAR).selected.is_some());
    }

    /// Button bar segments added, deleted and made active, web viewer
    /// options: written and read back.
    #[test]
    fn button_bar_and_web_viewer_edits() {
        let Some(mut d) = objects_demo() else { return };
        let i = d
            .layouts
            .iter()
            .position(|l| l.name == "Objects Demo")
            .unwrap();
        let mut ds = Designer::new(&d, i);
        let bar = ds
            .layout
            .objects
            .iter()
            .find(|o| o.kind == kind::BUTTON_BAR)
            .unwrap()
            .id;
        let segs = ds.segments(bar);
        assert_eq!(segs.len(), 3);
        ds.add_segment(bar);
        let segs = ds.segments(bar);
        assert_eq!(segs.len(), 4);
        // Equal widths over the bar (360 pt).
        let w: Vec<f64> = segs
            .iter()
            .map(|id| {
                ds.object(*id)
                    .map_or(0.0, |o| o.bounds.right - o.bounds.left)
            })
            .collect();
        assert_eq!(w, [90.0; 4]);
        ds.object_mut(bar).unwrap().selected = Some(segs[1]);
        ds.delete_segment(bar, segs[1]);
        assert_eq!(ds.object(bar).unwrap().selected, Some(segs[0]));
        let new_seg = segs[3];
        ds.object_mut(new_seg).unwrap().label = Some(compile(&d, "Get ( LayoutName )").unwrap());
        ds.object_mut(bar).unwrap().selected = Some(new_seg);
        let web = ds
            .layout
            .objects
            .iter()
            .find(|o| o.url.is_some())
            .unwrap()
            .id;
        let e = ds.object_mut(web).unwrap().external.as_mut().unwrap();
        e.1 = (e.1 | ly::web::FIND_MODE) & !ly::web::STATUS;
        ds.apply(&mut d);
        let bytes = written(&d);
        // `BASECRAFT_BAR_DEMO=<out.fmp12>` keeps it for `fms_check.sh`.
        if let Some(p) = std::env::var_os("BASECRAFT_BAR_DEMO") {
            std::fs::write(p, &bytes).unwrap();
        }
        let f = fmp12::File::from_bytes(bytes).unwrap();
        let l = ly::layouts(&f)
            .unwrap()
            .into_iter()
            .find(|l| l.name == "Objects Demo")
            .unwrap();
        let back: Vec<&LayoutObject> = l.objects.iter().filter(|o| o.parent == Some(bar)).collect();
        assert_eq!(back.len(), 3);
        assert_eq!(
            l.objects.iter().find(|o| o.id == bar).unwrap().selected,
            Some(new_seg)
        );
        let s = back.iter().find(|o| o.id == new_seg).unwrap();
        assert_eq!(
            calc_text(&d, s.label.as_ref().unwrap()),
            "Get ( LayoutName )"
        );
        assert_eq!(
            l.objects.iter().find(|o| o.id == web).unwrap().external,
            Some((ly::WEB_VIEWER, 0x8007))
        );
    }

    /// Chart Setup on the charts fixture: title, series name, type and
    /// source written back; an empty series is refused.
    #[test]
    fn chart_setup_is_saved() {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fmp12/tests/fixtures/charts.fmp12"
        );
        let file = fmp12::File::open(p).unwrap();
        let db = engine::Database::load(&file).unwrap();
        let layouts = db.layouts.clone();
        let mut d = Doc {
            name: "charts".into(),
            db,
            layouts,
            file,
            path: None,
            triggers: Default::default(),
        };
        let i = d.layouts.iter().position(|l| l.id == 1).unwrap();
        let mut ds = Designer::new(&d, i);
        ds.open_chart_setup(2, &d);
        let mut e = ds.extra.chart.take().unwrap();
        assert_eq!(e.title, "\"Two series\"");
        assert_eq!(e.series.len(), 2);
        e.series[0].0.clear();
        assert!(e.build(&d).is_err());
        e = ChartEdit::read(&d, 2, ds.object(2).unwrap().chart.as_deref().unwrap());
        e.title = "\"Sales \" & Get ( FoundCount )".into();
        e.series[1].1 = "\"Costs\"".into();
        e.x_title.clear();
        e.chart.kind = Kind::Area;
        e.chart.source = Source::CurrentRecord;
        let c = e.build(&d).unwrap();
        ds.set_chart(2, c);
        assert!(ds.changed());
        ds.apply(&mut d);
        let out = fmp12::save::save_layouts(&d.file, &d.db.layouts).unwrap();
        let f = fmp12::File::from_bytes(out).unwrap();
        let back = fmp12::charts::charts(&f).unwrap();
        let c = &back[&(1, 2)];
        assert_eq!(
            calc_text(&d, c.title.as_ref().unwrap()),
            "\"Sales \" & Get ( FoundCount )"
        );
        assert_eq!(calc_text(&d, c.y[1].name.as_ref().unwrap()), "\"Costs\"");
        assert_eq!((c.kind, c.source), (Kind::Area, Source::CurrentRecord));
        assert!(c.x_title.is_none());
    }

    /// `BASECRAFT_OBJECTS_DEMO=<out.fmp12>`: writes the demo for
    /// `tools/oracle/layout_check.ps1`.
    #[test]
    fn objects_demo_file() {
        let Some(out) = std::env::var_os("BASECRAFT_OBJECTS_DEMO") else {
            return;
        };
        let Some(d) = objects_demo() else { return };
        std::fs::write(out, written(&d)).unwrap();
    }

    /// The app on the Objects Demo layout of the demo file, as written.
    fn demo_app() -> Option<crate::App> {
        let d = objects_demo()?;
        let mut app = crate::App::default();
        app.open("Assets".into(), written(&d));
        let doc = app.doc.as_mut()?;
        doc.path = None;
        app.layout = doc.layouts.iter().position(|l| l.name == "Objects Demo")?;
        Some(app)
    }

    fn demo_layout(app: &crate::App) -> Layout {
        app.doc.as_ref().unwrap().layouts[app.layout].clone()
    }

    /// Browse mode: formats, conditional formats, the popover opened, the
    /// second slide, the web viewer's confirmation, Tab following the tab
    /// order. `BASECRAFT_SHOTS=<dir>` saves the screens.
    #[test]
    fn objects_in_browse_mode() {
        use egui_kittest::kittest::Queryable;
        let Some(app) = demo_app() else { return };
        let l = demo_layout(&app);
        let mut h = crate::ui_tests::harness_for(app);
        h.run_steps(4);
        crate::ui_tests::shot(&mut h, "l7_01_browse");
        // The popover opens with its button.
        let pop = l
            .objects
            .iter()
            .find(|o| o.kind == kind::POPOVER)
            .unwrap()
            .id;
        h.state_mut().objs.popover = Some(pop);
        h.run_steps(2);
        crate::ui_tests::shot(&mut h, "l7_02_popover");
        h.state_mut().objs.popover = None;
        // The second slide.
        let sc = l
            .objects
            .iter()
            .find(|o| o.kind == kind::SLIDE_CONTROL)
            .unwrap();
        let second = l
            .objects
            .iter()
            .filter(|o| o.parent == Some(sc.id))
            .map(|o| o.id)
            .nth(1)
            .unwrap();
        h.state_mut().tabs.insert(sc.id, second);
        h.run_steps(2);
        crate::ui_tests::shot(&mut h, "l7_03_slide");
        // Tab goes from the name to the price (the tab order).
        let (name, price) = (l.tab_order[0], l.tab_order[1]);
        h.state_mut().objs.focus = Some(name);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(h.state().objs.focus, Some(price));
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(h.state().objs.focus, Some(name));
        crate::ui_tests::shot(&mut h, "l7_04_tab");
        // "Open in Browser…" asks first; Cancel leaves the page closed.
        h.get_by_label(tr("Open in Browser…")).click();
        h.run_steps(4);
        assert!(
            h.state()
                .objs
                .open_url
                .as_deref()
                .is_some_and(|u| u.starts_with("https://example.com/?q="))
        );
        crate::ui_tests::shot(&mut h, "l7_05_url");
        h.get_by_label(tr("Cancel")).click();
        h.run_steps(2);
        assert!(h.state().objs.open_url.is_none());
    }

    /// Layout mode: the objects drawn, the Data tab's formats, the
    /// Conditional Formatting and Set Tab Order dialogs.
    #[test]
    fn objects_in_layout_mode() {
        use egui_kittest::kittest::Queryable;
        let Some(mut app) = demo_app() else { return };
        let l = demo_layout(&app);
        app.enter_layout();
        let price = l.tab_order[1];
        let name = l.tab_order[0];
        let mut h = crate::ui_tests::harness_for(app);
        h.run_steps(3);
        crate::ui_tests::shot(&mut h, "l7_10_layout");
        if let Some(ds) = h.state_mut().designer.as_mut() {
            ds.select([price]);
            ds.tab = super::super::Tab::Data;
        }
        h.run_steps(3);
        crate::ui_tests::shot(&mut h, "l7_11_data_formats");
        if let Some(ds) = h.state_mut().designer.as_mut() {
            ds.select([name]);
        }
        h.get_by_label(tr("Format")).click();
        h.run_steps(2);
        h.get_by_label(tr("Conditional Formatting…")).click();
        h.run_steps(3);
        assert!(
            h.state().designer.as_ref().is_some_and(|d| d
                .extra
                .cond
                .as_ref()
                .is_some_and(|c| c.1.len() == 2))
        );
        crate::ui_tests::shot(&mut h, "l7_12_conditions");
        h.get_by_label(tr("Cancel")).click();
        h.run_steps(2);
        h.get_by_label(tr("Arrange")).click();
        h.run_steps(2);
        h.get_by_label(tr("Set Tab Order…")).click();
        h.run_steps(3);
        crate::ui_tests::shot(&mut h, "l7_13_tab_order");
        // Moving the price first and OK changes the layout's tab order.
        if let Some(ds) = h.state_mut().designer.as_mut()
            && let Some((order, at)) = ds.extra.tabs.as_mut()
        {
            *at = 1;
            assert_eq!(order[1], price);
        }
        h.get_by_label("▲").click();
        h.run_steps(2);
        h.get_by_label(tr("OK")).click();
        h.run_steps(2);
        let ds = h.state().designer.as_ref().unwrap();
        assert_eq!(ds.layout.tab_order[..2], [price, name]);
    }
}
