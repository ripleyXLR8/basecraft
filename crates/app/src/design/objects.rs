//! Layout mode for popovers, slide controls, button bars and web viewers
//! (Insert menu and tools), data formatting, placeholders, tooltips and
//! "hide object when" (Inspector), conditional formatting (Format menu)
//! and the tab order (Arrange › Set Tab Order…).

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Rect, Stroke, StrokeKind, Vec2};
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
    ui.collapsing(tr("Number"), |ui| {
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
mod tests {
    use super::*;

    #[test]
    fn number_kinds_round_trip() {
        for k in 0..6 {
            assert_eq!(number_kind(with_number_kind(0x0A0C, k)), k);
        }
        // Currency keeps the leading symbol and fixed digits.
        assert_eq!(with_number_kind(0x0900, 3), 0x0A04);
    }
}
