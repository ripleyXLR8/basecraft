//! File › Manage › Custom Functions…: the list of the file's custom
//! functions (name, parameters, availability, folders as FileMaker shows
//! them), and an editor for one function. OK writes the changes into the
//! file in memory (`fmp12::schema_edit`), saved with the file.

use eframe::egui;
use fmp12::custom_functions::Kind;
use fmp12::schema_edit::CustomFunctionSpec;

use crate::{App, i18n};

/// UI strings: English keys, French as FileMaker 26's French UI writes them.
pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Custom Functions…" => "Fonctions personnalisées…",
        "Manage Custom Functions" => "Gérer les fonctions personnalisées",
        "Function Name" => "Nom de fonction",
        "Parameters" => "Paramètres",
        "Availability" => "Disponibilité",
        "All accounts" => "Tous les comptes",
        "Only accounts assigned full access privileges" => "Comptes avec accès intégral uniquement",
        "New" => "Nouveau",
        "Duplicate" => "Dupliquer",
        "Delete" => "Supprimer",
        "Function name:" => "Nom de la fonction :",
        "Function parameters (separated by ;):" => "Paramètres de la fonction (séparés par ;) :",
        "Calculation:" => "Calcul :",
        "OK" => "OK",
        "Cancel" => "Annuler",
        "function(s)" => "fonction(s)",
        "Folder" => "Dossier",
        _ => key,
    }
}

/// One function being edited.
#[derive(Clone, Debug, Default)]
struct Row {
    /// `None` for a function added in the dialog.
    id: Option<u32>,
    folder: Option<String>,
    name: String,
    /// As FileMaker lists them: `a ; b`.
    params: String,
    formula: String,
    full_access_only: bool,
    changed: bool,
}

impl Row {
    fn spec(&self) -> CustomFunctionSpec {
        CustomFunctionSpec {
            name: self.name.trim().to_string(),
            params: self
                .params
                .split(';')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
            formula: self.formula.clone(),
            full_access_only: self.full_access_only,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Dialog {
    rows: Vec<Row>,
    selected: Option<usize>,
    deleted: Vec<u32>,
    error: Option<String>,
}

fn dialog_id() -> egui::Id {
    egui::Id::NULL.with("manage_custom_functions")
}

impl App {
    /// Opens Manage Custom Functions for the open file.
    pub(crate) fn open_custom_functions(&mut self, ctx: &egui::Context) {
        let Some(d) = self.doc.as_ref() else { return };
        let cf = &d.db.custom_functions;
        let mut folders: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        for f in &cf.list {
            match f.kind {
                Kind::Folder => folders.push(f.name.clone()),
                Kind::FolderEnd => {
                    folders.pop();
                }
                Kind::Function => rows.push(Row {
                    id: Some(f.id),
                    folder: folders.last().cloned(),
                    name: f.name.clone(),
                    params: f.params.join(" ; "),
                    formula: cf.formulas.get(&f.id).cloned().unwrap_or_default(),
                    full_access_only: f.full_access_only,
                    changed: false,
                }),
            }
        }
        let dlg = Dialog {
            selected: (!rows.is_empty()).then_some(0),
            rows,
            ..Default::default()
        };
        ctx.data_mut(|m| m.insert_temp(dialog_id(), dlg));
    }

    /// Shows Manage Custom Functions while it is open.
    pub(crate) fn custom_functions_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = ctx.data(|m| m.get_temp::<Dialog>(dialog_id())) else {
            return;
        };
        let title = self.doc.as_ref().map_or(String::new(), |d| {
            format!("{} “{}”", t("Manage Custom Functions"), d.name)
        });
        let mut close = None;
        egui::Modal::new(dialog_id()).show(ctx, |ui| {
            ui.set_min_width(720.0);
            ui.heading(title);
            ui.label(format!("{} {}", dlg.rows.len(), t("function(s)")));
            list(ui, &mut dlg);
            ui.horizontal(|ui| {
                if ui.button(t("New")).clicked() {
                    dlg.rows.push(Row {
                        name: format!("Function{}", dlg.rows.len() + 1),
                        formula: "\"\"".into(),
                        changed: true,
                        ..Default::default()
                    });
                    dlg.selected = Some(dlg.rows.len() - 1);
                }
                let sel = dlg.selected.filter(|i| *i < dlg.rows.len());
                if ui
                    .add_enabled(sel.is_some(), egui::Button::new(t("Duplicate")))
                    .clicked()
                    && let Some(i) = sel
                {
                    let mut r = dlg.rows[i].clone();
                    r.id = None;
                    r.name = format!("{} Copy", r.name).replace(' ', "_");
                    r.changed = true;
                    dlg.rows.insert(i + 1, r);
                    dlg.selected = Some(i + 1);
                }
                if ui
                    .add_enabled(sel.is_some(), egui::Button::new(t("Delete")))
                    .clicked()
                    && let Some(i) = sel
                {
                    let r = dlg.rows.remove(i);
                    dlg.deleted.extend(r.id);
                    dlg.selected = (!dlg.rows.is_empty()).then(|| i.min(dlg.rows.len() - 1));
                }
            });
            ui.separator();
            if let Some(r) = dlg.selected.and_then(|i| dlg.rows.get_mut(i)) {
                editor(ui, r);
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(egui::Color32::RED, e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    close = Some(false);
                }
                if ui.button(t("OK")).clicked() {
                    close = Some(true);
                }
            });
        });
        let done = match close {
            Some(false) => true,
            Some(true) => match self.apply_custom_functions(&dlg) {
                Ok(()) => true,
                Err(e) => {
                    dlg.error = Some(e);
                    false
                }
            },
            None => false,
        };
        ctx.data_mut(|m| {
            if done {
                m.remove::<Dialog>(dialog_id());
            } else {
                m.insert_temp(dialog_id(), dlg);
            }
        });
    }

    /// Writes the dialog's changes: deletions, then new functions, then
    /// changes (a function may call one added or renamed in the same
    /// session: those that fail are tried again once the others are in).
    fn apply_custom_functions(&mut self, dlg: &Dialog) -> Result<(), String> {
        let Some(d) = self.doc.as_mut() else {
            return Ok(());
        };
        let before = (
            d.db.clone(),
            fmp12::File::from_bytes(d.file.bytes().to_vec()),
        );
        let result = (|| -> Result<(), String> {
            for id in &dlg.deleted {
                d.db.delete_custom_function(&mut d.file, *id)
                    .map_err(|e| e.to_string())?;
            }
            let mut ids = Vec::new();
            for r in &dlg.rows {
                let id = match r.id {
                    Some(id) => id,
                    None => {
                        let mut spec = r.spec();
                        spec.formula = "\"\"".into();
                        d.db.add_custom_function(&mut d.file, &spec)
                            .map_err(|e| format!("{}: {e}", r.name))?
                    }
                };
                ids.push(id);
            }
            let mut pending: Vec<(u32, &Row)> = ids
                .into_iter()
                .zip(&dlg.rows)
                .filter(|(_, r)| r.changed || r.id.is_none())
                .collect();
            for round in 0..2 {
                let mut failed = Vec::new();
                for (id, r) in pending {
                    if let Err(e) = d.db.set_custom_function(&mut d.file, id, &r.spec()) {
                        if round == 1 {
                            return Err(format!("{}: {e}", r.name));
                        }
                        failed.push((id, r));
                    }
                }
                pending = failed;
            }
            Ok(())
        })();
        if result.is_err()
            && let Ok(f) = before.1
        {
            d.db = before.0;
            d.file = f;
        } else if result.is_ok() && (!dlg.deleted.is_empty() || dlg.rows.iter().any(|r| r.changed))
        {
            self.dirty = true;
        }
        result
    }
}

/// The functions, by folder, as FileMaker lists them.
fn list(ui: &mut egui::Ui, dlg: &mut Dialog) {
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            egui::Grid::new("custom_function_list")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong(t("Function Name"));
                    ui.strong(t("Parameters"));
                    ui.strong(t("Availability"));
                    ui.end_row();
                    let mut folder: Option<&str> = None;
                    for (i, r) in dlg.rows.iter().enumerate() {
                        if r.folder.as_deref() != folder {
                            folder = r.folder.as_deref();
                            if let Some(f) = folder {
                                ui.label(format!("{}: {f}", t("Folder")));
                                ui.end_row();
                            }
                        }
                        let indent = if r.folder.is_some() { "    " } else { "" };
                        let picked = dlg.selected == Some(i);
                        if ui
                            .selectable_label(picked, format!("{indent}{}", r.name))
                            .clicked()
                        {
                            dlg.selected = Some(i);
                        }
                        ui.label(&r.params);
                        ui.label(if r.full_access_only {
                            t("Only accounts assigned full access privileges")
                        } else {
                            t("All accounts")
                        });
                        ui.end_row();
                    }
                });
        });
}

/// Name, parameters, calculation and availability of one function.
fn editor(ui: &mut egui::Ui, r: &mut Row) {
    let mut changed = false;
    egui::Grid::new("custom_function_editor")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(t("Function name:"));
            changed |= ui
                .add(egui::TextEdit::singleline(&mut r.name).id_salt("cf_name"))
                .changed();
            ui.end_row();
            ui.label(t("Function parameters (separated by ;):"));
            changed |= ui
                .add(egui::TextEdit::singleline(&mut r.params).id_salt("cf_params"))
                .changed();
            ui.end_row();
        });
    ui.label(t("Calculation:"));
    changed |= ui
        .add(
            egui::TextEdit::multiline(&mut r.formula)
                .id_salt("cf_formula")
                .code_editor()
                .desired_rows(6)
                .desired_width(f32::INFINITY),
        )
        .changed();
    ui.horizontal(|ui| {
        ui.label(t("Availability"));
        changed |= ui
            .radio_value(&mut r.full_access_only, false, t("All accounts"))
            .changed();
        changed |= ui
            .radio_value(
                &mut r.full_access_only,
                true,
                t("Only accounts assigned full access privileges"),
            )
            .changed();
    });
    r.changed |= changed;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// The dialog through egui's input: a new function, its calculation,
    /// OK; the file then evaluates it.
    #[test]
    fn new_function_through_the_dialog() {
        let mut app = App::default();
        let bytes = fmp12::schema_edit::new_file("Notes").unwrap();
        app.open("cf".into(), bytes);
        let mut h = Harness::builder()
            .with_size([900.0, 700.0])
            .with_context_setup(|ctx| {
                ctx.add_font_provider(std::sync::Arc::new(
                    egui_system_font_provider::SystemFontProvider::new(),
                ));
            })
            .build_eframe(move |cc| {
                cc.egui_ctx.set_visuals(egui::Visuals::light());
                app
            });
        h.run_steps(2);
        let ctx = h.ctx.clone();
        h.state_mut().open_custom_functions(&ctx);
        h.run_steps(2);
        h.get_by_label(t("New")).click();
        h.run_steps(2);
        let edit = |h: &mut Harness<'_, App>, id: &str, text: &str| {
            let mut dlg = h.ctx.data(|m| m.get_temp::<Dialog>(dialog_id())).unwrap();
            let r = dlg.rows.last_mut().unwrap();
            match id {
                "name" => r.name = text.into(),
                "params" => r.params = text.into(),
                _ => r.formula = text.into(),
            }
            h.ctx.data_mut(|m| m.insert_temp(dialog_id(), dlg));
        };
        edit(&mut h, "name", "Twice");
        edit(&mut h, "params", "x");
        edit(&mut h, "formula", "x * 2");
        h.run_steps(2);
        h.get_by_label(t("OK")).click();
        h.run_steps(3);
        assert!(h.ctx.data(|m| m.get_temp::<Dialog>(dialog_id())).is_none());
        let d = h.state().doc.as_ref().unwrap();
        assert_eq!(d.db.custom_functions.list[0].name, "Twice");
        assert_eq!(
            d.db.eval_source("Twice ( 21 )", d.db.tables[0].id, 0, None)
                .to_text(),
            "42"
        );
        assert!(h.state().dirty);
    }
}
