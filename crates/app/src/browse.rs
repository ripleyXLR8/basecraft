//! Records and View menu commands added for M5: the Sort Records dialog
//! (field list; ascending, descending or custom order by value list;
//! keep records in sorted order), Unsort, Go to Record…, Saved Finds
//! (kept for the session), Revert Record…, View as Form / List / Table,
//! Zoom In / Out and the status toolbar.

use std::collections::BTreeMap;

use eframe::egui;
use fmp12::scripts::{FieldRef, SortOrder};

use crate::{App, Mode, View, i18n, tr};

/// UI strings: English keys, French as FileMaker 26 shows them.
pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Sort Records…" => "Trier les enregistrements…",
        "Sort Records" => "Trier les enregistrements",
        "Unsort Records" => "Annuler le tri",
        "Unsort" => "Annuler le tri",
        "Go to Record…" => "Atteindre l'enregistrement…",
        "Go to Record" => "Atteindre l'enregistrement",
        "Go to record number:" => "Atteindre l'enregistrement n° :",
        "Saved Finds" => "Recherches enregistrées",
        "Save Current Find" => "Enregistrer la recherche actuelle",
        "Clear Saved Finds" => "Effacer les recherches enregistrées",
        "Name:" => "Nom :",
        "Revert Record…" => "Rétablir l'enregistrement…",
        "Revert record to last saved version?" => {
            "Rétablir l'enregistrement à la dernière version enregistrée ?"
        }
        "Revert" => "Rétablir",
        "Move" => "Déplacer",
        "Clear All" => "Tout effacer",
        "Sort" => "Trier",
        "OK" => "OK",
        "Cancel" => "Annuler",
        "Sort Order" => "Ordre de tri",
        "Ascending order" => "Ordre croissant",
        "Descending order" => "Ordre décroissant",
        "Custom order based on value list" => "Ordre personnalisé selon la liste de valeurs",
        "Keep records in sorted order" => "Conserver l'ordre de tri des enregistrements",
        "View as Form" => "Afficher sous forme de formulaire",
        "View as List" => "Afficher sous forme de liste",
        "View as Table" => "Afficher sous forme de tableau",
        "Zoom In" => "Zoom avant",
        "Zoom Out" => "Zoom arrière",
        "Status Toolbar" => "Barre d'outils d'état",
        k => tr(k),
    }
}

/// Find requests as the app keeps them: criteria by field, omit.
type Requests = Vec<(BTreeMap<u32, String>, bool)>;

/// One sort key of the dialog: field and order.
type Key = (u32, SortOrder);

pub struct SortDialog {
    pub order: Vec<Key>,
    /// Field picked in the left list, key picked in the right one.
    pick: Option<u32>,
    sel: Option<usize>,
    /// Order given to moved fields (and to the selected key).
    kind: SortOrder,
    keep: bool,
    /// Opened by Sort Records [With dialog] in a script.
    for_script: bool,
}

#[derive(Default)]
pub struct Dialogs {
    pub sort: Option<SortDialog>,
    goto: Option<String>,
    revert: bool,
    /// Saved Finds of the session: name and requests.
    saved: Vec<(String, Requests)>,
    save_name: Option<String>,
    /// Last sort (Sort Records dialog), re-applied when "keep records in
    /// sorted order" is on.
    sort_keys: Vec<Key>,
    keep_sorted: bool,
    /// Values of the current record before its first change (Revert).
    snapshot: Option<(u32, usize, engine::Record)>,
    was_editing: bool,
    /// The Sort dialog a script opened was closed: OK or not.
    pub script_sort: Option<bool>,
    /// Last text the engine copied, given to the OS clipboard.
    clip: Option<String>,
    zoom: u32,
    /// Install OnTimer Script: when the timer last fired (egui time).
    timer_at: Option<f64>,
}

impl App {
    /// Records menu entries added for M5.
    pub fn records_menu_more(&mut self, ui: &mut egui::Ui) {
        let browse = self.doc.is_some() && self.mode() == Mode::Browse;
        ui.separator();
        if ui
            .add_enabled(browse, egui::Button::new(t("Sort Records…")))
            .clicked()
        {
            self.open_sort(false);
            ui.close();
        }
        if ui
            .add_enabled(browse, egui::Button::new(t("Unsort Records")))
            .clicked()
        {
            self.unsort();
            ui.close();
        }
        if ui
            .add_enabled(browse, egui::Button::new(t("Go to Record…")))
            .clicked()
        {
            self.m5.goto = Some(String::new());
            ui.close();
        }
        ui.add_enabled_ui(browse, |ui| {
            ui.menu_button(t("Saved Finds"), |ui| self.saved_finds_items(ui));
        });
        let editing = self.editing.is_some();
        if ui
            .add_enabled(browse && editing, egui::Button::new(t("Revert Record…")))
            .clicked()
        {
            self.m5.revert = true;
            ui.close();
        }
    }

    /// Saved Finds: Save Current Find, the saved finds, Clear.
    pub fn saved_finds_items(&mut self, ui: &mut egui::Ui) {
        let has = !self.requests.iter().all(|r| r.0.is_empty());
        if ui
            .add_enabled(has, egui::Button::new(t("Save Current Find")))
            .clicked()
        {
            self.m5.save_name = Some(self.find_name());
            ui.close();
        }
        ui.separator();
        let mut run = None;
        for (i, (name, _)) in self.m5.saved.iter().enumerate() {
            if ui.button(name).clicked() {
                run = Some(i);
            }
        }
        if let Some(i) = run {
            self.run_saved_find(i);
            ui.close();
        }
        if !self.m5.saved.is_empty() {
            ui.separator();
            if ui.button(t("Clear Saved Finds")).clicked() {
                self.m5.saved.clear();
                ui.close();
            }
        }
    }

    /// View menu entries added for M5.
    pub fn view_menu_more(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        for (v, label) in [
            (View::Form, "View as Form"),
            (View::List, "View as List"),
            (View::Table, "View as Table"),
        ] {
            if ui.selectable_label(self.view == v, t(label)).clicked() {
                if self.view != v {
                    self.view = v;
                    self.fire(fmp12::triggers::event::VIEW_CHANGE);
                }
                ui.close();
            }
        }
        ui.separator();
        let mut on = self.win.toolbar;
        if ui
            .add(
                egui::Button::new(t("Status Toolbar"))
                    .selected(on)
                    .shortcut_text("Ctrl+Alt+S"),
            )
            .clicked()
        {
            on = !on;
            self.win.toolbar = on;
            ui.close();
        }
        self.customize_toolbar_menu(ui);
        ui.separator();
        if ui
            .add(egui::Button::new(t("Zoom In")).shortcut_text("F3"))
            .clicked()
        {
            self.win.zoom = zoom_step(self.win.zoom, true);
            ui.close();
        }
        if ui
            .add(egui::Button::new(t("Zoom Out")).shortcut_text("Shift+F3"))
            .clicked()
        {
            self.win.zoom = zoom_step(self.win.zoom, false);
            ui.close();
        }
    }

    /// Called once per frame before the layout is drawn: dialogs, the
    /// Revert snapshot, "keep sorted", the clipboard, the zoom, the
    /// debugger.
    pub fn m5_frame(&mut self, ctx: &egui::Context) {
        // Revert Record: the record's values before its first change.
        let editing = self.editing.is_some();
        if !editing && let (Some(t), Some(r)) = (self.table(), self.current_record()) {
            let rec = self
                .doc
                .as_ref()
                .and_then(|d| d.db.table(t)?.records.get(r).cloned());
            self.m5.snapshot = rec.map(|x| (t, r, x));
        }
        // Keep records in sorted order: sorted again after each commit.
        if self.m5.was_editing && !editing && self.m5.keep_sorted {
            let keys = self.m5.sort_keys.clone();
            self.apply_sort(&keys);
        }
        self.m5.was_editing = editing;
        // Cut / Copy steps: to the OS clipboard.
        if let Some(c) = self.engine.as_ref().and_then(|e| e.clipboard.clone())
            && self.m5.clip.as_ref() != Some(&c)
        {
            ctx.copy_text(c.clone());
            self.m5.clip = Some(c);
        }
        // Install OnTimer Script: the script runs every interval while
        // no other script runs (as FileMaker, it waits for idle).
        match self.engine.as_ref().and_then(|e| e.timer.clone()) {
            Some(tm) => {
                let now = ctx.input(|i| i.time);
                let since = *self.m5.timer_at.get_or_insert(now);
                if now - since >= tm.interval {
                    if self.active.is_none() && self.trigger_queue.is_empty() {
                        self.trigger_queue.push_back((tm.script, tm.parameter));
                    }
                    self.m5.timer_at = Some(now);
                }
                let left = (since + tm.interval - now).max(0.05);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
            }
            None => self.m5.timer_at = None,
        }
        // Zoom (Set Zoom Level, View › Zoom).
        // ponytail: the whole UI zooms, menus included; zooming only the
        // layout needs a scaled painter in the drawing code.
        if self.m5.zoom != self.win.zoom {
            self.m5.zoom = self.win.zoom;
            ctx.set_zoom_factor(self.win.zoom.clamp(25, 400) as f32 / 100.0);
        }
        self.sort_ui(ctx);
        self.goto_ui(ctx);
        self.saved_find_ui(ctx);
        self.revert_ui(ctx);
        self.debugger_window(ctx);
        self.script_ui_frame(ctx);
    }

    /// Opens the Sort Records dialog on the last sort order.
    pub fn open_sort(&mut self, for_script: bool) {
        if !for_script && !self.commit() {
            return;
        }
        self.m5.sort = Some(SortDialog {
            order: self.m5.sort_keys.clone(),
            pick: None,
            sel: None,
            kind: SortOrder::Ascending,
            keep: self.m5.keep_sorted,
            for_script,
        });
    }

    /// Records › Unsort Records: creation order.
    pub fn unsort(&mut self) {
        if !self.commit() {
            return;
        }
        let Some(tb) = self.table() else { return };
        self.found(tb).sort_unstable();
        self.sorted.insert(tb, false);
        self.current.insert(tb, 0);
        self.last_sort.clear();
    }

    /// Sorts the found set by `keys` (fields of the current table).
    pub fn apply_sort(&mut self, keys: &[Key]) {
        let Some(tb) = self.table() else { return };
        let Some(to) = self
            .doc
            .as_ref()
            .and_then(|d| d.layouts.get(self.layout))
            .map(|l| (l.table_occurrence & 0xFFF) as u32)
        else {
            return;
        };
        let refs: Vec<(FieldRef, SortOrder)> = keys
            .iter()
            .map(|(f, o)| {
                let r = FieldRef {
                    to,
                    field: *f,
                    repetition: 1,
                };
                (r, *o)
            })
            .collect();
        let mut found = self.found(tb).clone();
        if let Some(d) = &self.doc {
            script::sort(&d.db, tb, &mut found, &refs);
        }
        self.found.insert(tb, found);
        self.sorted.insert(tb, !keys.is_empty());
        self.current.insert(tb, 0);
        self.last_sort = keys
            .iter()
            .map(|(f, o)| {
                let o = match o {
                    SortOrder::Descending => engine::Order::Descending,
                    _ => engine::Order::Ascending,
                };
                (*f, o)
            })
            .collect();
    }

    fn sort_ui(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.m5.sort.take() else {
            return;
        };
        let Some(tb) = self.table() else { return };
        let Some(d) = self.doc.as_ref() else { return };
        let fields: Vec<(u32, String)> =
            d.db.table(tb)
                .map(|x| x.fields.iter().map(|f| (f.id, f.name.clone())).collect())
                .unwrap_or_default();
        let lists: Vec<(u32, String)> =
            d.db.value_lists
                .iter()
                .map(|v| (v.id, v.name.clone()))
                .collect();
        let name = |id: u32| {
            fields
                .iter()
                .find(|f| f.0 == id)
                .map_or(String::new(), |f| f.1.clone())
        };
        // None: still open; Some(None): cancel; Some(Some(keys)): sort.
        let mut done: Option<Option<Vec<Key>>> = None;
        let mut unsort = false;
        egui::Modal::new(egui::Id::NULL.with("sort_records")).show(ctx, |ui| {
            ui.heading(t("Sort Records"));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sort_fields")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.set_min_width(180.0);
                            for (id, n) in &fields {
                                let r = ui.selectable_label(st.pick == Some(*id), n);
                                if r.clicked() {
                                    st.pick = Some(*id);
                                }
                                if r.double_clicked() {
                                    st.order.retain(|k| k.0 != *id);
                                    st.order.push((*id, st.kind));
                                }
                            }
                        });
                });
                ui.vertical(|ui| {
                    if ui.button(t("Move")).clicked()
                        && let Some(f) = st.pick
                    {
                        st.order.retain(|k| k.0 != f);
                        st.order.push((f, st.kind));
                    }
                    if ui.button(t("Clear All")).clicked() {
                        st.order.clear();
                        st.sel = None;
                    }
                });
                ui.vertical(|ui| {
                    ui.strong(t("Sort Order"));
                    ui.set_min_width(200.0);
                    for (i, (f, o)) in st.order.iter().enumerate() {
                        let arrow = match o {
                            SortOrder::Ascending => "\u{2191}",
                            SortOrder::Descending => "\u{2193}",
                            SortOrder::Custom(_) => "\u{2195}",
                        };
                        if ui
                            .selectable_label(st.sel == Some(i), format!("{arrow} {}", name(*f)))
                            .clicked()
                        {
                            st.sel = Some(i);
                            st.kind = *o;
                        }
                    }
                });
            });
            ui.separator();
            let before = st.kind;
            ui.radio_value(&mut st.kind, SortOrder::Ascending, t("Ascending order"));
            ui.radio_value(&mut st.kind, SortOrder::Descending, t("Descending order"));
            ui.horizontal(|ui| {
                let custom = matches!(st.kind, SortOrder::Custom(_));
                if ui
                    .radio(custom, t("Custom order based on value list"))
                    .clicked()
                    && !custom
                    && let Some(v) = lists.first()
                {
                    st.kind = SortOrder::Custom(v.0);
                }
                if let SortOrder::Custom(cur) = st.kind {
                    let label = lists
                        .iter()
                        .find(|v| v.0 == cur)
                        .map_or("", |v| v.1.as_str());
                    egui::ComboBox::from_id_salt("sort_vl")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, n) in &lists {
                                ui.selectable_value(&mut st.kind, SortOrder::Custom(*id), n);
                            }
                        });
                }
            });
            if st.kind != before
                && let Some(k) = st.sel.and_then(|i| st.order.get_mut(i))
            {
                k.1 = st.kind;
            }
            ui.checkbox(&mut st.keep, t("Keep records in sorted order"));
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(t("Sort")).clicked() {
                    done = Some(Some(st.order.clone()));
                }
                if ui.button(t("Unsort")).clicked() {
                    unsort = true;
                    done = Some(None);
                }
                if ui.button(t("Cancel")).clicked() {
                    done = Some(None);
                }
            });
        });
        let Some(result) = done else {
            self.m5.sort = Some(st);
            return;
        };
        if st.for_script {
            self.m5.script_sort = Some(result.is_some());
        }
        if unsort {
            self.unsort();
        }
        if let Some(keys) = result {
            self.m5.keep_sorted = st.keep;
            self.m5.sort_keys = keys.clone();
            self.apply_sort(&keys);
        }
    }

    fn goto_ui(&mut self, ctx: &egui::Context) {
        let Some(mut n) = self.m5.goto.take() else {
            return;
        };
        let Some(tb) = self.table() else { return };
        let count = self.found(tb).len();
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("go_to_record")).show(ctx, |ui| {
            ui.heading(t("Go to Record"));
            ui.horizontal(|ui| {
                ui.label(t("Go to record number:"));
                let r = ui.text_edit_singleline(&mut n);
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
            });
            ui.label(format!("1 – {count}"));
            ui.horizontal(|ui| {
                if ui.button(t("OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            None => self.m5.goto = Some(n),
            Some(true) => {
                if let Ok(k) = n.trim().parse::<usize>()
                    && (1..=count).contains(&k)
                    && self.commit()
                {
                    self.current.insert(tb, k - 1);
                    self.fire(fmp12::triggers::event::RECORD_LOAD);
                }
            }
            Some(false) => {}
        }
    }

    /// Default name of a saved find: its first criteria, as FileMaker does.
    fn find_name(&self) -> String {
        let Some(d) = self.doc.as_ref() else {
            return String::new();
        };
        let tbl = self.table().and_then(|t| d.db.table(t));
        self.requests
            .iter()
            .flat_map(|(c, _)| c.iter())
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(f, v)| {
                let n = tbl
                    .and_then(|x| x.field(*f))
                    .map_or("", |x| x.name.as_str());
                format!("{n}: {v}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn saved_find_ui(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.m5.save_name.take() else {
            return;
        };
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("save_find")).show(ctx, |ui| {
            ui.heading(t("Save Current Find"));
            ui.horizontal(|ui| {
                ui.label(t("Name:"));
                ui.text_edit_singleline(&mut name);
            });
            ui.horizontal(|ui| {
                if ui.button(t("OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            None => self.m5.save_name = Some(name),
            Some(true) => {
                let reqs = self.requests.clone();
                self.m5.saved.retain(|s| s.0 != name);
                self.m5.saved.push((name, reqs));
            }
            Some(false) => {}
        }
    }

    /// Records › Saved Finds › a saved find: performs it again.
    pub fn run_saved_find(&mut self, i: usize) {
        let Some((_, reqs)) = self.m5.saved.get(i).cloned() else {
            return;
        };
        if reqs.is_empty() || !self.commit() {
            return;
        }
        self.requests = reqs;
        self.req = 0;
        (self.request, self.omit) = self.requests[0].clone();
        self.perform_find();
    }

    /// Saves the current find under `name` (tests).
    #[cfg(test)]
    pub fn save_find(&mut self, name: &str) {
        let reqs = self.requests.clone();
        self.m5.saved.push((name.to_string(), reqs));
    }

    fn revert_ui(&mut self, ctx: &egui::Context) {
        if !self.m5.revert {
            return;
        }
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("revert_record")).show(ctx, |ui| {
            ui.label(t("Revert record to last saved version?"));
            ui.horizontal(|ui| {
                if ui.button(t("Revert")).clicked() {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        if let Some(yes) = done {
            self.m5.revert = false;
            if yes {
                self.revert_record();
            }
        }
    }

    /// Puts back the values the record had before its first change.
    pub fn revert_record(&mut self) {
        let (Some((t, r)), Some((st, sr, rec))) = (self.editing, self.m5.snapshot.clone()) else {
            return;
        };
        if (t, r) != (st, sr) || !self.record_revert() {
            return;
        }
        if let Some(tb) = self
            .doc
            .as_mut()
            .and_then(|d| d.db.tables.iter_mut().find(|x| x.id == t))
            && let Some(x) = tb.records.get_mut(r)
        {
            *x = rec;
        }
        self.editing = None;
    }
}

/// FileMaker's zoom levels.
const LEVELS: [u32; 8] = [25, 50, 75, 100, 150, 200, 300, 400];

pub(crate) fn zoom_step(z: u32, up: bool) -> u32 {
    let z = if z == 0 { 100 } else { z };
    if up {
        LEVELS.iter().copied().find(|l| *l > z).unwrap_or(400)
    } else {
        LEVELS.iter().rev().copied().find(|l| *l < z).unwrap_or(25)
    }
}
