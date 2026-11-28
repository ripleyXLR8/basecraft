//! Editing in Browse mode: Edit › Undo / Redo of field edits (a field
//! being typed in has egui's own undo, as FileMaker's), the Insert menu
//! (Current Date, Time, User Name, From Index, From Last Visited Record)
//! at the cursor of the field last edited, and Edit › Preferences.

use eframe::egui;

use crate::toolbar::{Focus, insert_at_cursor, last_focus};
use crate::{App, Mode, i18n, tr};

fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Undo" => "Annuler",
        "Redo" => "Rétablir",
        "Can't Undo" => "Impossible d'annuler",
        "Can't Redo" => "Impossible de rétablir",
        "Current Date" => "Date du jour",
        "Current Time" => "Heure actuelle",
        "Current User Name" => "Nom de l'utilisateur actuel",
        "From Index…" => "Index…",
        "From Last Visited Record" => "Du dernier enregistrement consulté",
        "Insert from Index" => "Insérer depuis l'index",
        "OK" => "OK",
        "Cancel" => "Annuler",
        k => tr(k),
    }
}

/// A field edit, for Undo and Redo: table, record, value key, before, after.
#[derive(Clone, Debug)]
pub struct Edit {
    pub table: u32,
    pub record: usize,
    pub key: u32,
    pub before: String,
    pub after: String,
}

#[derive(Default)]
pub struct State {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Record shown now and the one shown before (From Last Visited).
    now: Option<(u32, usize)>,
    last: Option<(u32, usize)>,
    /// Insert › From Index…: the field, its values, the one picked.
    index: Option<(Focus, Vec<String>, Option<usize>)>,
}

fn new_edits_id() -> egui::Id {
    egui::Id::NULL.with("bc_new_edits")
}

/// A field edit done while drawing (collected by [`App::editing_frame`]).
pub fn record(ctx: &egui::Context, e: Edit) {
    if e.before != e.after {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<Edit>>(new_edits_id())
                .push(e)
        });
    }
}

impl App {
    /// Each frame: edits made while drawing join the undo list; the
    /// record shown is remembered (From Last Visited Record).
    pub fn editing_frame(&mut self, ctx: &egui::Context) {
        let new: Vec<Edit> = ctx.data_mut(|d| d.remove_temp(new_edits_id()).unwrap_or_default());
        if !new.is_empty() {
            self.edits.undo.extend(new);
            self.edits.redo.clear();
        }
        if self.mode() == Mode::Browse
            && let (Some(t), Some(r)) = (self.table(), self.current_record())
            && self.edits.now != Some((t, r))
        {
            self.edits.last = self.edits.now.replace((t, r));
        }
        self.index_dialog(ctx);
    }

    fn undo_redo(&mut self, redo: bool) {
        let e = if redo {
            self.edits.redo.pop()
        } else {
            self.edits.undo.pop()
        };
        let Some(e) = e else { return };
        let Some(d) = self.doc.as_mut() else { return };
        if d.db
            .table(e.table)
            .is_none_or(|x| x.records.len() <= e.record)
        {
            return;
        }
        let v = if redo { &e.after } else { &e.before };
        d.db.set_values(e.table, e.record, &[(e.key, v.clone())]);
        self.dirty = true;
        // The record undone becomes the current one, as in FileMaker.
        if self.table() == Some(e.table)
            && let Some(pos) = self.found(e.table).iter().position(|r| *r == e.record)
        {
            self.current.insert(e.table, pos);
        }
        if redo {
            self.edits.undo.push(e);
        } else {
            self.edits.redo.push(e);
        }
    }

    /// Edit menu outside Layout mode: Undo, Redo, Find/Replace, Preferences.
    pub fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let browse = self.mode() == Mode::Browse;
        let (u, r) = (!self.edits.undo.is_empty(), !self.edits.redo.is_empty());
        let undo =
            egui::Button::new(t(if u { "Undo" } else { "Can't Undo" })).shortcut_text("Ctrl+Z");
        if ui.add_enabled(browse && u, undo).clicked() {
            ui.close();
            self.undo_redo(false);
        }
        let redo =
            egui::Button::new(t(if r { "Redo" } else { "Can't Redo" })).shortcut_text("Ctrl+Y");
        if ui.add_enabled(browse && r, redo).clicked() {
            ui.close();
            self.undo_redo(true);
        }
        ui.separator();
        self.edit_menu_items(ui);
        ui.separator();
        self.prefs_menu(ui);
    }

    /// The field last edited, if it shows on the current record.
    fn insert_target(&mut self, ctx: &egui::Context) -> Option<Focus> {
        let f = last_focus(ctx)?;
        let (t, rec, _) = f.record?;
        (self.mode() == Mode::Browse
            && self.table() == Some(t)
            && self.current_record() == Some(rec))
        .then_some(f)
    }

    /// Insert menu entries of Browse mode.
    pub fn insert_menu_items(&mut self, ui: &mut egui::Ui) {
        let on = self.insert_target(ui.ctx()).is_some();
        ui.separator();
        for (what, key) in [
            ("Current Date", "Ctrl+-"),
            ("Current Time", "Ctrl+;"),
            ("Current User Name", "Ctrl+Shift+N"),
        ] {
            if ui
                .add_enabled(on, egui::Button::new(t(what)).shortcut_text(key))
                .clicked()
            {
                ui.close();
                self.insert_now(ui.ctx(), what);
            }
        }
        ui.separator();
        if ui
            .add_enabled(
                on,
                egui::Button::new(t("From Index…")).shortcut_text("Ctrl+I"),
            )
            .clicked()
        {
            ui.close();
            self.open_index(ui.ctx());
        }
        let last = on
            && self
                .edits
                .last
                .is_some_and(|(t, _)| Some(t) == self.table());
        if ui
            .add_enabled(
                last,
                egui::Button::new(t("From Last Visited Record")).shortcut_text("Ctrl+'"),
            )
            .clicked()
        {
            ui.close();
            self.insert_last_visited(ui.ctx());
        }
    }

    /// Inserts `text` at the cursor of the field last edited.
    fn insert_text(&mut self, ctx: &egui::Context, f: Focus, text: &str) {
        let Some((t, rec, key)) = f.record else {
            return;
        };
        let Some(d) = self.doc.as_mut() else { return };
        if !d.db.can_edit_field(t, rec, engine::split_rep_key(key).0) {
            return;
        }
        let Some(tbl) = d.db.tables.iter_mut().find(|x| x.id == t) else {
            return;
        };
        let Some(r) = tbl.records.get_mut(rec) else {
            return;
        };
        let s = r.values.entry(key).or_default();
        let before = s.clone();
        insert_at_cursor(ctx, f.id, s, text);
        let after = s.clone();
        d.db.touch(t, rec);
        self.dirty = true;
        self.editing = Some((t, rec));
        record(
            ctx,
            Edit {
                table: t,
                record: rec,
                key,
                before,
                after,
            },
        );
    }

    /// Insert › Current Date, Current Time, Current User Name.
    fn insert_now(&mut self, ctx: &egui::Context, what: &str) {
        let Some(f) = self.insert_target(ctx) else {
            return;
        };
        let flag = match what {
            "Current Date" => "CurrentDate",
            "Current Time" => "CurrentTime",
            _ => "UserName",
        };
        let Some(d) = self.doc.as_ref() else { return };
        let Some((t, rec, _)) = f.record else { return };
        let v =
            d.db.eval_source(&format!("Get ( {flag} )"), t, 0, Some(rec));
        let text = engine::stored_text(&v, &d.db.locale);
        self.insert_text(ctx, f, &text);
    }

    /// Insert › From Index…: the field's values in all records, sorted.
    fn open_index(&mut self, ctx: &egui::Context) {
        let Some(f) = self.insert_target(ctx) else {
            return;
        };
        let (Some((t, _, key)), Some(d)) = (f.record, self.doc.as_ref()) else {
            return;
        };
        let Some(tbl) = d.db.table(t) else { return };
        let mut values: Vec<String> = (0..tbl.records.len())
            .map(|r| value_of(&d.db, t, r, key))
            .filter(|v| !v.is_empty())
            .collect();
        values.sort_by_key(|v| v.to_lowercase());
        values.dedup();
        self.edits.index = Some((f, values, None));
    }

    fn index_dialog(&mut self, ctx: &egui::Context) {
        let Some((f, values, pick)) = self.edits.index.as_mut() else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::NULL.with("bc_insert_index")).show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.heading(t("Insert from Index"));
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for (i, v) in values.iter().enumerate() {
                        let r = ui.selectable_label(*pick == Some(i), v);
                        if r.clicked() {
                            *pick = Some(i);
                        }
                        if r.double_clicked() {
                            answer = Some(true);
                        }
                    }
                });
            ui.horizontal(|ui| {
                if ui.button(t("OK")).clicked() {
                    answer = Some(true);
                }
                if ui.button(t("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let f = *f;
        let v = pick.and_then(|i| values.get(i).cloned());
        match (answer, v) {
            (Some(true), Some(v)) => {
                self.edits.index = None;
                self.insert_text(ctx, f, &v);
            }
            (Some(_), _) => self.edits.index = None,
            _ => {}
        }
    }

    /// Insert › From Last Visited Record: the field's value there.
    fn insert_last_visited(&mut self, ctx: &egui::Context) {
        let Some(f) = self.insert_target(ctx) else {
            return;
        };
        let (Some((t, _, key)), Some((lt, lr)), Some(d)) =
            (f.record, self.edits.last, self.doc.as_ref())
        else {
            return;
        };
        if lt != t {
            return;
        }
        let v = value_of(&d.db, t, lr, key);
        self.insert_text(ctx, f, &v);
    }

    /// Ctrl+Z / Ctrl+Y outside a field, and the Insert menu's shortcuts.
    pub fn edit_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        if self.mode() != Mode::Browse || self.active.is_some() {
            return;
        }
        let key = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if ctx.memory(|m| m.focused().is_none()) {
            if key(M::COMMAND, Key::Z) {
                self.undo_redo(false);
            } else if key(M::COMMAND, Key::Y) {
                self.undo_redo(true);
            }
        }
        if self.insert_target(ctx).is_none() {
            return;
        }
        if key(M::COMMAND, Key::Minus) {
            self.insert_now(ctx, "Current Date");
        } else if key(M::COMMAND, Key::Semicolon) {
            self.insert_now(ctx, "Current Time");
        } else if key(M::COMMAND | M::SHIFT, Key::N) {
            self.insert_now(ctx, "Current User Name");
        } else if key(M::COMMAND, Key::I) {
            self.open_index(ctx);
        } else if key(M::COMMAND, Key::Quote) {
            self.insert_last_visited(ctx);
        }
    }
}

/// Text of the value at `key` (field and repetition) of a record.
fn value_of(db: &engine::Database, t: u32, r: usize, key: u32) -> String {
    let (field, rep) = engine::split_rep_key(key);
    db.value_rep(t, r, field, rep).to_text()
}
