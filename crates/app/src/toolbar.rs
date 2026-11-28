//! The status toolbar and layout bar as in FileMaker 26: the record book
//! (previous / next, record number, slider), found and total counts and
//! (Sorted), Show All, New / Delete Record, Find, Sort, Share, Quick Find;
//! in Find mode the requests, Perform Find (Constrain / Extend), Cancel
//! Find, Include / Omit and Insert › Operators. Also the mode shortcuts
//! (Ctrl+B, F, L, U, S…) and the Requests menu of Find mode.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32};
use fmp12::triggers::event;

use crate::{App, Mode, View, design, i18n, tr};

pub fn tx(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Found" => "Trouvés",
        "Records" => "Enregistrements",
        "Requests" => "Requêtes",
        "Find Requests" => "Requêtes de recherche",
        "Delete Request" => "Supprimer la requête",
        "Duplicate Request" => "Dupliquer la requête",
        "Add New Request" => "Ajouter une requête",
        "Cancel Find" => "Annuler la recherche",
        "Constrain Found Set" => "Restreindre l'ensemble trouvé",
        "Extend Found Set" => "Étendre l'ensemble trouvé",
        "Matching records:" => "Enregistrements concordants :",
        "Include" => "Inclure",
        "Insert:" => "Insérer :",
        "Operators" => "Opérateurs",
        "Quick Find" => "Recherche rapide",
        "No records match this find criteria." => {
            "Aucun enregistrement ne correspond à ces critères de recherche."
        }
        "Share" => "Partager",
        "Sharing a file needs FileMaker Server." => {
            "Le partage d'un fichier demande FileMaker Server."
        }
        "Record number" => "Numéro d'enregistrement",
        "Saved Finds" => "Recherches enregistrées",
        "Recent Finds" => "Recherches récentes",
        "Customize Status Toolbar…" => "Personnaliser la barre d'outils…",
        "Labels" => "Libellés",
        "Exact match" => "mot entier (ou vide)",
        "Match entire field" => "rubrique entière",
        "Find duplicate values" => "rechercher les doublons",
        "Less than" => "inférieur à",
        "Less than or equal" => "inférieur ou égal à",
        "Greater than" => "supérieur à",
        "Greater than or equal" => "supérieur ou égal à",
        "Range" => "intervalle",
        "Today's date" => "date du jour",
        "Invalid date or time" => "date/heure incorrecte",
        "One character" => "l'un des caractères",
        "One digit" => "l'un des chiffres",
        "Zero or more characters" => "zéro, un ou plusieurs caractères",
        "Literal text" => "expression (à partir du début du mot)",
        k => tr(k),
    }
}

/// Insert › Operators of Find mode: operator and description.
pub const OPERATORS: [(&str, &str); 14] = [
    ("=", "Exact match"),
    ("==", "Match entire field"),
    ("!", "Find duplicate values"),
    ("<", "Less than"),
    ("≤", "Less than or equal"),
    (">", "Greater than"),
    ("≥", "Greater than or equal"),
    ("...", "Range"),
    ("//", "Today's date"),
    ("?", "Invalid date or time"),
    ("@", "One character"),
    ("#", "One digit"),
    ("*", "Zero or more characters"),
    ("\"\"", "Literal text"),
];

#[derive(Default)]
pub struct State {
    /// Record number being typed in the record book.
    goto: Option<String>,
    /// Quick Find text.
    pub quick: String,
    /// Customize Status Toolbar: buttons hidden, labels shown.
    hidden: Vec<&'static str>,
    no_labels: bool,
}

/// Field that had the keyboard last (Insert menu, Insert › Operators): its
/// text edit, and where its text lives.
#[derive(Clone, Copy, Debug)]
pub struct Focus {
    pub id: egui::Id,
    /// Browse mode: (table, record, value key); Find mode: field id.
    pub record: Option<(u32, usize, u32)>,
    pub find: Option<u32>,
}

pub fn focus_key() -> egui::Id {
    egui::Id::NULL.with("bc_field_focus")
}

/// Remembers the field being edited.
pub fn set_focus(ctx: &egui::Context, f: Focus) {
    ctx.data_mut(|d| d.insert_temp(focus_key(), f));
}

pub fn last_focus(ctx: &egui::Context) -> Option<Focus> {
    ctx.data(|d| d.get_temp(focus_key()))
}

/// Inserts `ins` at the cursor of text edit `id` showing `text` (at the
/// end when it never had the cursor), and puts the cursor after it.
pub fn insert_at_cursor(ctx: &egui::Context, id: egui::Id, text: &mut String, ins: &str) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let n = text.chars().count();
    let (a, b) = state.cursor.char_range().map_or((n, n), |r| {
        let (x, y) = (r.primary.index.0.min(n), r.secondary.index.0.min(n));
        (x.min(y), x.max(y))
    });
    let byte = |i: usize| text.char_indices().nth(i).map_or(text.len(), |(p, _)| p);
    let (ba, bb) = (byte(a), byte(b));
    text.replace_range(ba..bb, ins);
    let at = egui::text::CCursor::new(a + ins.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(at)));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Toolbar icons, drawn (no font has them all).
#[derive(Clone, Copy)]
enum Icon {
    ShowAll,
    New,
    Delete,
    Find,
    FindNew,
    FindDelete,
    Perform,
    Cancel,
    Sort,
    Share,
}

fn paint_icon(p: &egui::Painter, c: egui::Pos2, icon: Icon, color: Color32) {
    use egui::{Rect, Stroke, pos2, vec2};
    let s = Stroke::new(1.4, color);
    let square = |p: &egui::Painter| {
        p.rect_filled(Rect::from_center_size(c, vec2(12.0, 12.0)), 1.0, color);
    };
    let glass = |p: &egui::Painter| {
        p.circle_stroke(c + vec2(-1.5, -1.5), 4.5, s);
        p.line_segment([c + vec2(1.8, 1.8), c + vec2(5.5, 5.5)], s);
    };
    let bar = |p: &egui::Painter, x: f32, y: f32, plus: bool, col: Color32| {
        let at = c + vec2(x, y);
        let st = Stroke::new(1.6, col);
        p.line_segment([at - vec2(3.0, 0.0), at + vec2(3.0, 0.0)], st);
        if plus {
            p.line_segment([at - vec2(0.0, 3.0), at + vec2(0.0, 3.0)], st);
        }
    };
    match icon {
        Icon::ShowAll => {
            for d in [vec2(-2.0, -2.0), vec2(2.0, 2.0)] {
                p.rect_stroke(
                    Rect::from_center_size(c + d, vec2(10.0, 10.0)),
                    1.0,
                    s,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Icon::New | Icon::Delete => {
            square(p);
            bar(p, 0.0, 0.0, matches!(icon, Icon::New), Color32::WHITE);
        }
        Icon::Find => glass(p),
        Icon::FindNew | Icon::FindDelete => {
            glass(p);
            bar(p, 5.5, -5.5, matches!(icon, Icon::FindNew), color);
        }
        Icon::Perform => {
            p.circle_stroke(c, 6.5, s);
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-2.0, -3.5),
                    c + vec2(3.5, 0.0),
                    c + vec2(-2.0, 3.5),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Cancel => {
            p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
            p.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], s);
        }
        Icon::Sort => {
            p.line_segment([c + vec2(-3.0, -6.0), c + vec2(-3.0, 6.0)], s);
            p.line_segment([c + vec2(-6.0, 3.0), c + vec2(-3.0, 6.0)], s);
            p.line_segment([c + vec2(0.0, 3.0), c + vec2(-3.0, 6.0)], s);
            for (i, w) in [2.0, 4.0, 6.0].into_iter().enumerate() {
                let y = c.y - 5.0 + i as f32 * 4.0;
                p.line_segment([pos2(c.x + 2.0, y), pos2(c.x + 2.0 + w, y)], s);
            }
        }
        Icon::Share => {
            p.rect_stroke(
                Rect::from_min_max(c + vec2(-5.0, -1.0), c + vec2(5.0, 6.0)),
                0.0,
                s,
                egui::StrokeKind::Inside,
            );
            p.line_segment([c + vec2(0.0, -7.0), c + vec2(0.0, 3.0)], s);
            p.line_segment([c + vec2(-3.0, -4.0), c + vec2(0.0, -7.0)], s);
            p.line_segment([c + vec2(3.0, -4.0), c + vec2(0.0, -7.0)], s);
        }
    }
}

/// A toolbar button: icon above, label below (FileMaker's look).
fn tool(ui: &mut egui::Ui, icon: Icon, label: &'static str, on: bool, labels: bool) -> bool {
    let text = tx(label);
    let font = egui::FontId::proportional(11.5);
    let width = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), Color32::BLACK)
        .size()
        .x;
    let size = if labels {
        egui::vec2(width.max(28.0) + 8.0, 38.0)
    } else {
        egui::vec2(28.0, 28.0)
    };
    let sense = if on {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(size, sense);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, text));
    let color = if !on {
        Color32::from_gray(175)
    } else if r.hovered() {
        Color32::from_rgb(0, 90, 180)
    } else {
        ui.visuals().text_color()
    };
    let c = egui::pos2(rect.center().x, rect.top() + 12.0);
    paint_icon(
        ui.painter(),
        if labels { c } else { rect.center() },
        icon,
        color,
    );
    if labels {
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.bottom() - galley.size().y,
            ),
            galley,
            color,
        );
    }
    r.on_hover_text(text).clicked()
}

impl App {
    fn total(&self, t: u32) -> usize {
        self.doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |x| x.records.len())
    }

    /// Goes to record `pos` of the found set (0-based), as the book does.
    pub fn go_to(&mut self, pos: usize) {
        let Some(t) = self.table() else { return };
        let n = self.found(t).len();
        let cur = self.current.get(&t).copied().unwrap_or(0);
        if n == 0 || pos == cur || !self.commit() {
            return;
        }
        self.current.insert(t, pos.min(n - 1));
        self.fire(event::RECORD_LOAD);
    }

    fn step_record(&mut self, forward: bool) {
        let Some(t) = self.table() else { return };
        let cur = self.current.get(&t).copied().unwrap_or(0);
        match (forward, self.mode()) {
            (_, Mode::Find) => {
                let n = self.requests.len().max(self.req + 1);
                if forward && self.req + 1 < n {
                    self.switch_request(self.req + 1);
                } else if !forward && self.req > 0 {
                    self.switch_request(self.req - 1);
                }
            }
            (_, Mode::Preview) => {
                if forward && self.page + 1 < self.pages {
                    self.page += 1;
                } else if !forward {
                    self.page = self.page.saturating_sub(1);
                }
            }
            (true, _) => self.go_to(cur + 1),
            (false, _) => self.go_to(cur.saturating_sub(1)),
        }
    }

    /// Status toolbar: Browse, Find and Preview modes (Layout mode shows
    /// the designer's tools).
    pub fn status_toolbar(&mut self, ui: &mut egui::Ui) {
        if matches!(
            self.active.as_ref().map(|a| &a.wait),
            Some(script::Suspend::Pause(_))
        ) {
            let now = ui.input(|i| i.time);
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(0, 90, 180), tr("Script paused"));
                if ui.button(tr("Continue")).clicked() {
                    self.answer(script::Answer::Continue, now);
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.answer(script::Answer::Cancel, now);
                }
            });
            ui.separator();
        }
        if let Some(ds) = self.designer.as_mut() {
            ui.horizontal(|ui| ds.tools(ui));
            return;
        }
        let Some(t) = self.table() else { return };
        let total = self.total(t);
        let found = self.found(t).len();
        let mode = self.mode();
        let labels = !self.bar.no_labels;
        let (pos, count) = match mode {
            Mode::Find => (self.req, self.requests.len().max(self.req + 1)),
            Mode::Preview => (self.page, self.pages.max(1)),
            _ => (*self.current.get(&t).unwrap_or(&0), found),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The book: previous, next.
            if ui
                .add_enabled(pos > 0, egui::Button::new("◀"))
                .on_hover_text(tr("Previous"))
                .clicked()
            {
                self.step_record(false);
            }
            if ui
                .add_enabled(pos + 1 < count, egui::Button::new("▶"))
                .on_hover_text(tr("Next"))
                .clicked()
            {
                self.step_record(true);
            }
            // Record number (Enter goes there) above the slider.
            ui.vertical(|ui| {
                let shown = if count == 0 { 0 } else { pos + 1 };
                let s = self.bar.goto.get_or_insert_with(|| shown.to_string());
                let r = ui.add(
                    egui::TextEdit::singleline(s)
                        .desired_width(56.0)
                        .id_salt("bc_record_number"),
                );
                let r = r.on_hover_text(tx("Record number"));
                if r.lost_focus() {
                    let n = s.trim().parse::<usize>().ok();
                    self.bar.goto = None;
                    if let Some(n) =
                        n.filter(|n| *n >= 1 && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        match mode {
                            Mode::Find => self.switch_request((n - 1).min(count - 1)),
                            Mode::Preview => self.page = (n - 1).min(count - 1),
                            _ => self.go_to(n - 1),
                        }
                    }
                } else if !r.has_focus() {
                    self.bar.goto = None;
                }
                if count > 1 {
                    let mut v = pos + 1;
                    let slider = egui::Slider::new(&mut v, 1..=count)
                        .show_value(false)
                        .trailing_fill(true);
                    if ui.add_sized([56.0, 12.0], slider).changed() {
                        match mode {
                            Mode::Find => self.switch_request(v - 1),
                            Mode::Preview => self.page = v - 1,
                            _ => self.go_to(v - 1),
                        }
                    }
                }
            });
            // Counts: found, total (Sorted / Unsorted).
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let small = |s: String| egui::RichText::new(s).size(11.0);
                match mode {
                    Mode::Find => {
                        ui.label(small(count.to_string()).strong());
                        ui.label(small(tr("Total").into()));
                        caption(ui, small(tx("Find Requests").into()));
                    }
                    Mode::Preview => {
                        ui.label(small(format!("{} {}/{count}", tr("Page"), pos + 1)).strong());
                    }
                    _ => {
                        let s = if self.sorted.get(&t).copied().unwrap_or(false) {
                            "Sorted"
                        } else {
                            "Unsorted"
                        };
                        if found < total {
                            ui.label(small(format!("{found} {}", tx("Found"))).strong());
                            ui.label(small(format!("{total} {} ({})", tr("Total"), tr(s))));
                        } else {
                            ui.label(small(total.to_string()).strong());
                            ui.label(small(format!("{} ({})", tr("Total"), tr(s))));
                        }
                        caption(ui, small(tx("Records").into()));
                    }
                }
            });
            ui.separator();
            let hidden = self.bar.hidden.clone();
            let shown = |k: &str| !hidden.contains(&k);
            match mode {
                Mode::Browse => {
                    if shown("Show All")
                        && tool(ui, Icon::ShowAll, "Show All", found < total, labels)
                    {
                        self.show_all();
                    }
                    if shown("New Record") && tool(ui, Icon::New, "New Record", true, labels) {
                        self.new_record();
                    }
                    if shown("Delete Record")
                        && tool(ui, Icon::Delete, "Delete Record", found > 0, labels)
                    {
                        self.confirm_delete();
                    }
                    ui.separator();
                    if shown("Find") {
                        if tool(ui, Icon::Find, "Find", true, labels) {
                            self.enter_find();
                        }
                        // Saved and recent finds, as FileMaker's ▾.
                        ui.menu_button("▾", |ui| self.saved_finds_menu(ui));
                    }
                    if shown("Sort") && tool(ui, Icon::Sort, "Sort", found > 0, labels) {
                        self.open_sort(false);
                    }
                    ui.separator();
                    if shown("Share") && tool(ui, Icon::Share, "Share", true, labels) {
                        self.messages.push((
                            String::new(),
                            tx("Sharing a file needs FileMaker Server.").into(),
                        ));
                    }
                    let quick = self.quick_find_enabled();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add_enabled(
                            quick,
                            egui::TextEdit::singleline(&mut self.bar.quick)
                                .hint_text(tx("Quick Find"))
                                .desired_width(180.0)
                                .id_salt("bc_quick_find"),
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let q = self.bar.quick.clone();
                            self.quick_find(&q);
                        }
                    });
                }
                Mode::Find => {
                    if tool(ui, Icon::FindNew, "New Request", true, labels) {
                        self.switch_request(count);
                    }
                    if tool(ui, Icon::FindDelete, "Delete Request", count > 1, labels) {
                        self.delete_request();
                    }
                    ui.separator();
                    if tool(ui, Icon::Perform, "Perform Find", true, labels) {
                        self.perform_find();
                    }
                    ui.menu_button("▾", |ui| {
                        if ui.button(tx("Constrain Found Set")).clicked() {
                            ui.close();
                            self.find_combined(Some(false));
                        }
                        if ui.button(tx("Extend Found Set")).clicked() {
                            ui.close();
                            self.find_combined(Some(true));
                        }
                    });
                    if tool(ui, Icon::Cancel, "Cancel Find", true, labels) {
                        self.cancel_find();
                    }
                    ui.separator();
                    ui.menu_button(tx("Saved Finds"), |ui| self.saved_finds_menu(ui));
                }
                Mode::Preview => {
                    if ui.button(tr("Exit Preview")).clicked() {
                        self.browse_mode();
                    }
                }
                Mode::Layout => {}
            }
            if self.dirty {
                ui.colored_label(Color32::from_rgb(180, 90, 0), tr("Unsaved changes"));
            }
        });
    }

    /// View › Customize Status Toolbar: the buttons shown, their labels.
    pub fn customize_toolbar_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tx("Customize Status Toolbar…"), |ui| {
            let mut labels = !self.bar.no_labels;
            if ui.checkbox(&mut labels, tx("Labels")).changed() {
                self.bar.no_labels = !labels;
            }
            ui.separator();
            for k in [
                "Show All",
                "New Record",
                "Delete Record",
                "Find",
                "Sort",
                "Share",
            ] {
                let mut on = !self.bar.hidden.contains(&k);
                if ui.checkbox(&mut on, tx(k)).changed() {
                    self.bar.hidden.retain(|x| *x != k);
                    if !on {
                        self.bar.hidden.push(k);
                    }
                }
            }
        });
    }

    /// The Find button's ▾ and the Saved Finds button: saved finds.
    fn saved_finds_menu(&mut self, ui: &mut egui::Ui) {
        self.saved_finds_items(ui);
    }

    /// Layout bar: layout pop-up, View As, Preview, Edit Layout; in Find
    /// mode Include / Omit and Insert › Operators.
    pub fn layout_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            return;
        };
        let names: Vec<String> = doc.layouts.iter().map(|l| l.name.clone()).collect();
        let mut chosen = None;
        if self.mode() == Mode::Layout {
            let mut then = None;
            ui.horizontal(|ui| {
                ui.label(tr("Layout:"));
                let current = names.get(self.layout).map_or("", |n| n.as_str());
                egui::ComboBox::from_id_salt("layout")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            if ui.selectable_label(i == self.layout, n).clicked()
                                && i != self.layout
                            {
                                then = Some(design::Then::Switch(i));
                            }
                        }
                    });
                if ui.button(tr("New Layout/Report")).clicked()
                    && let (Some(ds), Some(d)) = (self.designer.as_mut(), self.doc.as_ref())
                {
                    ds.open_new_layout(d);
                }
                if ui.button(tr("Layout Setup…")).clicked()
                    && let Some(ds) = self.designer.as_mut()
                {
                    ds.open_setup();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Exit Layout")).clicked() {
                        then = Some(design::Then::Exit);
                    }
                });
            });
            if let Some(t) = then {
                self.leave_layout(t);
            }
            return;
        }
        let find = self.mode() == Mode::Find;
        self.enforce_view();
        let views = self.views_allowed();
        let Some(doc) = &self.doc else {
            return;
        };
        // Layout Setup › "Include in layout menus" off: not listed.
        let hidden: Vec<bool> = doc
            .layouts
            .iter()
            .map(|l| l.flags & fmp12::layouts::setup::HIDDEN != 0)
            .collect();
        ui.horizontal(|ui| {
            ui.label(tr("Layout:"));
            let current = names.get(self.layout).map_or("", |n| n.as_str());
            egui::ComboBox::from_id_salt("layout")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if hidden.get(i) == Some(&true) && i != self.layout {
                            continue;
                        }
                        if ui.selectable_label(i == self.layout, n).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            ui.label(tr("View As:"));
            for ((v, label), ok) in [
                (View::Form, "Form"),
                (View::List, "List"),
                (View::Table, "Table"),
            ]
            .into_iter()
            .zip(views)
            {
                let b = egui::Button::selectable(self.view == v, tr(label));
                if ui.add_enabled(ok, b).clicked() && self.view != v {
                    self.view = v;
                    self.fire(event::VIEW_CHANGE);
                }
            }
            if find {
                ui.separator();
                ui.label(tx("Matching records:"));
                ui.selectable_value(&mut self.omit, false, tx("Include"));
                ui.selectable_value(&mut self.omit, true, tr("Omit"));
                ui.label(tx("Insert:"));
                ui.menu_button(format!("{} ▾", tx("Operators")), |ui| {
                    for (op, what) in OPERATORS {
                        if ui.button(format!("{op}   {}", tx(what))).clicked() {
                            ui.close();
                            self.insert_operator(ui.ctx(), op);
                        }
                    }
                });
                return;
            }
            let in_preview = self.mode() == Mode::Preview;
            if ui
                .add(egui::Button::selectable(in_preview, tr("Preview")))
                .clicked()
            {
                self.toggle_preview();
            }
            let mut edit = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                edit = ui.button(tr("Edit Layout")).clicked();
            });
            if edit {
                chosen = None;
                self.enter_layout();
            }
        });
        if let Some(i) = chosen.filter(|i| *i != self.layout && self.commit()) {
            self.fire(event::LAYOUT_EXIT);
            self.layout = i;
            self.fire(event::LAYOUT_ENTER);
            self.fire(event::RECORD_LOAD);
        }
    }

    pub fn toggle_preview(&mut self) {
        if self.mode() == Mode::Preview {
            if !self.mode_exit() {
                return;
            }
            self.mode = Some(Mode::Browse);
            self.fire(event::MODE_ENTER);
        } else if self.mode() != Mode::Layout && self.commit() && self.mode_exit() {
            self.mode = Some(Mode::Preview);
            self.page = 0;
            self.fire(event::MODE_ENTER);
        }
    }

    /// View › Browse Mode.
    pub fn browse_mode(&mut self) {
        match self.mode() {
            Mode::Layout => self.leave_layout(design::Then::Exit),
            Mode::Browse => {}
            _ if !self.mode_exit() => {}
            _ => {
                self.mode = Some(Mode::Browse);
                self.fire(event::MODE_ENTER);
            }
        }
    }

    fn cancel_find(&mut self) {
        if !self.mode_exit() {
            return;
        }
        self.mode = Some(Mode::Browse);
        self.fire(event::MODE_ENTER);
    }

    /// Requests › Delete Request.
    pub fn delete_request(&mut self) {
        self.switch_request(self.req);
        if self.requests.len() <= 1 {
            self.requests = vec![(BTreeMap::new(), false)];
            (self.request, self.omit) = (BTreeMap::new(), false);
            return;
        }
        self.requests.remove(self.req);
        self.req = self.req.min(self.requests.len() - 1);
        (self.request, self.omit) = self.requests[self.req].clone();
    }

    /// Requests › Duplicate Request.
    fn duplicate_request(&mut self) {
        self.switch_request(self.req);
        let r = (self.request.clone(), self.omit);
        self.requests.insert(self.req + 1, r);
        self.switch_request(self.req + 1);
    }

    /// Perform Find, Constrain Found Set (`Some(false)`) or Extend Found
    /// Set (`Some(true)`). No match: FileMaker's message, Find mode stays.
    pub fn find_combined(&mut self, extend: Option<bool>) {
        let Some(t) = self.table() else { return };
        let before = self.found(t).clone();
        let reqs = self.requests_now();
        if reqs.iter().all(|r| r.criteria.is_empty()) {
            return;
        }
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        let hits = engine::find_from(&d.db, (l.table_occurrence & 0xFFF) as u32, &reqs);
        let found: Vec<usize> = match extend {
            None => hits,
            Some(false) => before
                .iter()
                .copied()
                .filter(|r| hits.contains(r))
                .collect(),
            Some(true) => {
                let mut v = before.clone();
                v.extend(hits.iter().filter(|r| !before.contains(r)));
                v
            }
        };
        if found.is_empty() {
            self.messages.push((
                String::new(),
                tx("No records match this find criteria.").into(),
            ));
            return;
        }
        if !self.mode_exit() {
            return;
        }
        self.found.insert(t, found);
        self.sorted.insert(t, false);
        self.current.insert(t, 0);
        self.mode = Some(Mode::Browse);
        self.fire(event::MODE_ENTER);
        self.fire(event::RECORD_LOAD);
    }

    /// The requests of Find mode, the one being edited included.
    fn requests_now(&mut self) -> Vec<engine::Request> {
        self.switch_request(self.req);
        self.requests
            .iter()
            .map(|(c, omit)| engine::Request {
                criteria: c
                    .iter()
                    .filter(|(_, v)| !v.trim().is_empty())
                    .map(|(k, v)| (*k, v.clone()))
                    .collect(),
                omit: *omit,
            })
            .collect()
    }

    /// Quick Find: records where every word starts a word of one of the
    /// layout's fields (the layout's table), as FileMaker's search box.
    /// Fields whose "Include field for Quick Find" is off are skipped.
    pub fn quick_find(&mut self, text: &str) {
        let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() || !self.commit() {
            return;
        }
        let Some(t) = self.table() else { return };
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        let mut fields: Vec<u32> = l
            .objects
            .iter()
            .filter(|o| {
                o.kind == 1 && o.field_flags & fmp12::layouts::field_flags::NO_QUICK_FIND == 0
            })
            .filter_map(|o| {
                let (to, f, _) = o.field?;
                (d.db.table_of((to & 0xFFF) as u32)?.id == t).then_some(f as u32)
            })
            .collect();
        fields.sort_unstable();
        fields.dedup();
        let hits: Vec<usize> = (0..self.total(t))
            .filter(|&r| {
                let all: Vec<String> = fields
                    .iter()
                    .map(|f| d.db.value(t, r, *f).to_text().to_lowercase())
                    .collect();
                words.iter().all(|w| {
                    all.iter().any(|v| {
                        v.split(|c: char| !c.is_alphanumeric())
                            .any(|x| x.starts_with(w.as_str()))
                            || v.contains(w.as_str()) && !w.chars().all(char::is_alphanumeric)
                    })
                })
            })
            .collect();
        if hits.is_empty() {
            self.messages.push((
                String::new(),
                tx("No records match this find criteria.").into(),
            ));
            return;
        }
        self.found.insert(t, hits);
        self.sorted.insert(t, false);
        self.current.insert(t, 0);
        self.fire(event::RECORD_LOAD);
    }

    /// Insert › Operators: into the find field last edited.
    fn insert_operator(&mut self, ctx: &egui::Context, op: &str) {
        let Some(f) = last_focus(ctx) else { return };
        let Some(field) = f.find else { return };
        let s = self.request.entry(field).or_default();
        insert_at_cursor(ctx, f.id, s, op);
    }

    /// Shortcuts of the View menu and of Find mode.
    pub fn mode_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        if self.active.is_some() {
            return;
        }
        let key = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if key(M::COMMAND, Key::B) {
            self.browse_mode();
        } else if key(M::COMMAND, Key::F) {
            if self.mode() != Mode::Find {
                self.enter_find();
            }
        } else if key(M::COMMAND, Key::U) {
            if self.mode() != Mode::Preview {
                self.toggle_preview();
            }
        } else if key(M::COMMAND | M::ALT, Key::S) {
            self.win.toolbar = !self.win.toolbar;
        } else if self.mode() != Mode::Find && key(M::COMMAND, Key::S) {
            self.open_sort(false);
        } else if key(M::NONE, Key::F3) {
            self.win.zoom = crate::browse::zoom_step(self.win.zoom, true);
        } else if key(M::SHIFT, Key::F3) {
            self.win.zoom = crate::browse::zoom_step(self.win.zoom, false);
        } else if key(M::COMMAND, Key::ArrowUp) {
            self.step_record(false);
        } else if key(M::COMMAND, Key::ArrowDown) {
            self.step_record(true);
        }
        if self.mode() == Mode::Find {
            if key(M::COMMAND, Key::N) {
                let n = self.requests.len().max(self.req + 1);
                self.switch_request(n);
            } else if key(M::COMMAND, Key::D) {
                self.duplicate_request();
            } else if key(M::COMMAND, Key::E) {
                self.delete_request();
            } else if key(M::NONE, Key::Enter) {
                self.perform_find();
            }
        }
        // List and Table views: the arrows move between records when no
        // field has the keyboard.
        let free = ctx.memory(|m| m.focused().is_none());
        if free && self.mode() == Mode::Browse && self.view != View::Form {
            if key(M::NONE, Key::ArrowUp) {
                self.step_record(false);
            } else if key(M::NONE, Key::ArrowDown) {
                self.step_record(true);
            }
        }
    }

    /// The Requests menu, which replaces the Records menu in Find mode.
    pub fn requests_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &'static str, key: &str, on: bool| {
            ui.add_enabled(on, egui::Button::new(tx(label)).shortcut_text(key))
                .clicked()
        };
        let n = self.requests.len().max(self.req + 1);
        if item(ui, "Add New Request", "Ctrl+N", true) {
            ui.close();
            self.switch_request(n);
        }
        if item(ui, "Duplicate Request", "Ctrl+D", true) {
            ui.close();
            self.duplicate_request();
        }
        if item(ui, "Delete Request", "Ctrl+E", n > 1) {
            ui.close();
            self.delete_request();
        }
        ui.separator();
        if item(ui, "Show All Records", "Ctrl+J", true) {
            ui.close();
            self.show_all();
            self.cancel_find();
        }
        if item(ui, "Perform Find", "Enter", true) {
            ui.close();
            self.perform_find();
        }
        if item(ui, "Constrain Found Set", "", true) {
            ui.close();
            self.find_combined(Some(false));
        }
        if item(ui, "Extend Found Set", "", true) {
            ui.close();
            self.find_combined(Some(true));
        }
    }
}

/// Text drawn without a widget (not one more "Records" for the tests and
/// screen readers, which have the menu's).
fn caption(ui: &mut egui::Ui, text: egui::RichText) {
    let g = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let (r, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
    ui.painter().galley(r.min, g, ui.visuals().text_color());
}
