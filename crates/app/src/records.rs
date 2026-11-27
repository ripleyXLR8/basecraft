//! Records menu commands and the M8 dialogs: Edit › Find/Replace,
//! Records › Replace Field Contents, Omit Multiple, Modify Last Find,
//! Duplicate Record, Delete (All) Records with confirmations, File ›
//! Import Records (CSV, tab, Excel, FileMaker) with the field mapping
//! dialog, and File › Export Records with the field order dialog.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use eframe::egui;
use engine::export::Format;
use engine::replace::{Direction, FindReplace, Hit, ReplaceWith};
use fmp12::schema::FieldKind;

use crate::{App, Mode, i18n};

/// UI strings of these dialogs: English keys, French as FileMaker 26's
/// French UI writes them; other keys go to [`i18n::tr`].
pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Find/Replace…" => "Rechercher/Remplacer…",
        "Find/Replace" => "Rechercher et remplacer",
        "Find what:" => "Rechercher :",
        "Replace with:" => "Remplacer par :",
        "Direction:" => "Direction :",
        "Forward" => "Vers l'avant",
        "Backward" => "Vers l'arrière",
        "All" => "Toutes",
        "Match case" => "Respecter la casse",
        "Match whole word" => "Mot entier",
        "Search across:" => "Recherche sur :",
        "All records" => "Tous les enregistrements",
        "Current record" => "Enregistrement actif",
        "Search within:" => "Recherche dans :",
        "All fields" => "Toutes les rubriques",
        "Current field" => "Rubrique active",
        "Find Next" => "Suivant",
        "Replace & Find" => "Remplacer et rechercher",
        "Replace" => "Remplacer",
        "Replace All" => "Remplacer tout",
        "Close" => "Fermer",
        "No match found." => "Aucune correspondance trouvée.",
        "Replace Field Contents…" => "Remplacer le contenu de la rubrique…",
        "Replace Field Contents" => "Remplacer le contenu de la rubrique",
        "Field:" => "Rubrique :",
        "Replace with serial numbers:" => "Remplacer par num. de série :",
        "Initial value:" => "N° de départ :",
        "Increment by:" => "Incrément :",
        "Update serial number in Entry Options" => "Répercuter dans Options d'entrée",
        "Replace with calculated result:" => "Remplacer par le résultat du calcul :",
        "Perform auto-enter options while replacing" => {
            "Activer les options d'entrée automatique pour les rubriques"
        }
        "Cancel" => "Annuler",
        "Duplicate Record" => "Dupliquer l'enregistrement",
        "Delete All Records…" => "Supprimer tous les enregistrements…",
        "Delete" => "Supprimer",
        "Delete All" => "Tout supprimer",
        "Permanently delete this ENTIRE record?" => {
            "Supprimer définitivement cet enregistrement ENTIER ?"
        }
        "Omit Multiple…" => "Ignorer plusieurs…",
        "Omit Multiple Records" => "Ignorer plusieurs enregistrements",
        "Omit" => "Ignorer",
        "Number of records to omit:" => "Nombre d'enregistrements à ignorer :",
        "Modify Last Find" => "Modifier la dernière recherche",
        "Import Records" => "Importer des enregistrements",
        "File…" => "Fichier…",
        "Import Field Mapping" => "Définir l'ordre d'importation",
        "Set the import order of the target fields." => {
            "Définissez l'ordre d'importation des rubriques cibles."
        }
        "SOURCE" => "SOURCE",
        "TARGET" => "CIBLE",
        "Current Table" => "Table active",
        "Table:" => "Table :",
        "Source fields" => "Rubriques sources",
        "Mapping" => "Mise en correspondance",
        "Target fields" => "Rubriques cibles",
        "Matching names" => "Noms correspondants",
        "Creation order" => "Ordre de création",
        "Don't import first record (contains field names)" => {
            "Ne pas importer le premier enregistrement (noms de rubriques)"
        }
        "Add" => "Ajouter",
        "Update" => "Mettre à jour",
        "Add new records" => "Ajouter de nouveaux enregistrements",
        "Update matching records in the found set when match fields are equal" => {
            "Mettre à jour les enregistrements correspondants de l'ensemble trouvé lorsque les rubriques de correspondance sont égales"
        }
        "Replace the found set's data in record order" => {
            "Remplacer les données de l'ensemble trouvé dans l'ordre des enregistrements"
        }
        "Add remaining data as new records" => {
            "Ajouter les données restantes en tant que nouveaux enregistrements"
        }
        "Perform auto-enter options while importing" => {
            "Activer les options d'entrée automatique pour les rubriques pendant l'importation"
        }
        "Import" => "Importer",
        "Record" => "Enregistrement",
        "Don't import" => "Ne pas importer",
        "From source file" => "Depuis le fichier source",
        "of" => "sur",
        "Import Summary" => "Récapitulatif de l'importation",
        "Total records added / updated:" => "Total des enregistrements ajoutés/mis à jour :",
        "Total records skipped due to errors:" => {
            "Total des enregistrements ignorés en raison d'erreurs :"
        }
        "Export Records…" => "Exporter des enregistrements…",
        "Specify Field Order for Export" => "Ordre d'exportation des rubriques",
        "Select and move fields to export. To group data, select the fields to group by before moving other fields." => {
            "Sélectionnez et déplacez les rubriques à exporter. Si vous souhaitez regrouper des données, sélectionnez les rubriques d'après lesquelles le regroupement sera effectué avant de déplacer les autres rubriques."
        }
        "Current Layout" => "Modèle actif",
        "Field export order" => "Ordre d'exportation des rubriques",
        "Move" => "Ajouter",
        "Move All" => "Tout ajouter",
        "Clear All" => "Tout effacer",
        "Group by" => "Grouper par",
        "(Unsorted)" => "(Non triées)",
        "Options" => "Options",
        "Apply current layout's data formatting" => "Formater selon le modèle actif",
        "Type:" => "Type :",
        "Export" => "Exporter",
        "Comma-Separated Values" => "Valeurs séparées par des virgules",
        "Tab-Separated Values" => "Valeurs séparées par des tabulations",
        "Excel Workbooks" => "Classeurs Excel",
        "HTML Table" => "Tableau HTML",
        "XML" => "XML",
        "Merge" => "Fusion",
        other => i18n::tr(other),
    }
}

/// How a source column is imported (the symbol between the two lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrow {
    /// → import the column into the target field.
    Import,
    /// = match records on this field (update matching records).
    Match,
    /// — do not import the column.
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Add,
    /// Update existing records in the found set (row i → record i).
    Replace,
    /// Update matching records (match fields).
    Update,
}

struct Import {
    name: String,
    format: Format,
    /// FileMaker source: its tables (name, rows with a field-name row).
    tables: Vec<engine::import::SourceTable>,
    src_table: usize,
    rows: Vec<Vec<String>>,
    header: bool,
    /// Per source column: target field and arrow.
    targets: Vec<Option<u32>>,
    arrows: Vec<Arrow>,
    action: Action,
    add_remaining: bool,
    auto_enter: bool,
    /// Source record shown (0-based).
    sample: usize,
}

struct Export {
    format: Format,
    /// Export order: (table occurrence, field).
    fields: Vec<(u32, u32)>,
    /// Field selected in the left list.
    pick: Option<(u32, u32)>,
    /// The layout's occurrence.
    to: u32,
    /// Occurrence whose fields the left list shows (0: the layout's).
    browse: u32,
    formatted: bool,
    /// Sort fields to group by.
    group: Vec<bool>,
}

struct FindDialog {
    q: FindReplace,
    dir: Direction,
    current_only: bool,
    current_field: bool,
    last: Option<Hit>,
    status: String,
}

struct ReplaceContents {
    field: Option<u32>,
    kind: u8,
    value: String,
    initial: String,
    increment: String,
    update: bool,
    calc: String,
    auto_enter: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Delete,
    DeleteAll,
}

/// A file picked for import (asynchronous in the browser).
type Picked = Rc<RefCell<Option<(String, Vec<u8>)>>>;

#[derive(Default)]
pub struct Dialogs {
    find: Option<FindDialog>,
    replace: Option<ReplaceContents>,
    import: Option<Import>,
    export: Option<Export>,
    omit: Option<String>,
    confirm: Option<Confirm>,
    picked: Picked,
    /// Field object that last had the focus (Current field, Replace Field Contents).
    pub focus: Option<u64>,
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Csv => t("Comma-Separated Values"),
        Format::Tab => t("Tab-Separated Values"),
        Format::Xlsx => t("Excel Workbooks"),
        Format::Html => t("HTML Table"),
        Format::Xml => t("XML"),
        Format::Merge => t("Merge"),
        Format::Json => "JSON",
    }
}

fn extension(f: Format) -> &'static str {
    match f {
        Format::Csv => "csv",
        Format::Tab => "tab",
        Format::Xlsx => "xlsx",
        Format::Html => "htm",
        Format::Xml => "xml",
        Format::Merge => "mer",
        Format::Json => "json",
    }
}

impl App {
    /// Fields of the current layout's table on the layout, in layout order
    /// (top to bottom, left to right): what Find/Replace searches.
    fn layout_fields(&self) -> Vec<u32> {
        let (Some(d), Some(t)) = (self.doc.as_ref(), self.table()) else {
            return Vec::new();
        };
        let Some(l) = d.layouts.get(self.layout) else {
            return Vec::new();
        };
        let mut objs: Vec<_> = l
            .objects
            .iter()
            .filter(|o| o.kind == 1)
            .filter_map(|o| {
                let (to, f, _) = o.field?;
                let b = l.absolute(o);
                (d.db.table_of((to & 0xFFF) as u32)?.id == t).then_some((b.top, b.left, f as u32))
            })
            .collect();
        objs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut out: Vec<u32> = Vec::new();
        for (_, _, f) in objs {
            if !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }

    /// Field of the field object that last had the focus.
    fn focused_field(&self) -> Option<u32> {
        let d = self.doc.as_ref()?;
        let o = d
            .layouts
            .get(self.layout)?
            .objects
            .iter()
            .find(|o| Some(o.id) == self.m8.focus)?;
        Some(o.field?.1 as u32)
    }

    /// Record ids of every found set, to rebuild them after deletions.
    fn found_ids(&self) -> BTreeMap<u32, Vec<u32>> {
        let Some(d) = self.doc.as_ref() else {
            return BTreeMap::new();
        };
        self.found
            .iter()
            .filter_map(|(t, f)| {
                let tbl = d.db.table(*t)?;
                Some((
                    *t,
                    f.iter()
                        .filter_map(|&r| tbl.records.get(r).map(|x| x.id))
                        .collect(),
                ))
            })
            .collect()
    }

    fn restore_found(&mut self, ids: BTreeMap<u32, Vec<u32>>) {
        let Some(d) = self.doc.as_ref() else { return };
        for (t, list) in ids {
            let Some(tbl) = d.db.table(t) else { continue };
            let idx: BTreeMap<u32, usize> = tbl
                .records
                .iter()
                .enumerate()
                .map(|(i, r)| (r.id, i))
                .collect();
            let f: Vec<usize> = list.iter().filter_map(|id| idx.get(id).copied()).collect();
            let pos = self.current.get(&t).copied().unwrap_or(0);
            self.current.insert(t, pos.min(f.len().saturating_sub(1)));
            self.found.insert(t, f);
        }
    }

    /// Deletes records (indexes into the current table) and the related
    /// records their relationships delete; found sets keep the rest.
    fn delete_records(&mut self, records: &[usize]) {
        let Some(t) = self.table() else { return };
        let ids = self.found_ids();
        let Some(d) = self.doc.as_mut() else { return };
        let Some(tbl) = d.db.table(t) else { return };
        let rids: Vec<u32> = records
            .iter()
            .filter_map(|&r| tbl.records.get(r).map(|x| x.id))
            .collect();
        for id in rids {
            if let Some(i) =
                d.db.table(t)
                    .and_then(|x| x.records.iter().position(|r| r.id == id))
            {
                d.db.delete_record(t, i);
            }
        }
        self.editing = None;
        self.restore_found(ids);
        self.dirty = true;
    }

    /// Records › Delete Record…: asks first, as FileMaker does.
    pub fn confirm_delete(&mut self) {
        self.m8.confirm = Some(Confirm::Delete);
    }

    /// Records › Duplicate Record: the copy follows the original in the found set.
    pub fn duplicate_record(&mut self) {
        if !self.commit() {
            return;
        }
        let (Some(t), Some(src)) = (self.table(), self.current_record()) else {
            return;
        };
        let Some(r) = self
            .doc
            .as_mut()
            .and_then(|d| d.db.duplicate_record(t, src))
        else {
            return;
        };
        let pos = self.current.get(&t).copied().unwrap_or(0);
        let f = self.found(t);
        let at = (pos + 1).min(f.len());
        f.insert(at, r);
        self.current.insert(t, at);
        self.dirty = true;
        self.fire(fmp12::triggers::event::RECORD_LOAD);
    }

    /// Records › Omit Multiple: omits `n` records from the current one.
    fn omit_multiple(&mut self, n: usize) {
        let Some(t) = self.table() else { return };
        if !self.commit() {
            return;
        }
        let pos = self.current.get(&t).copied().unwrap_or(0);
        let f = self.found(t);
        let end = (pos + n).min(f.len());
        f.drain(pos.min(end)..end);
        let n = f.len();
        self.current.insert(t, pos.min(n.saturating_sub(1)));
    }

    /// Records › Modify Last Find: Find mode with the last find's requests.
    pub fn modify_last_find(&mut self) {
        if self.requests.is_empty() {
            self.enter_find();
            return;
        }
        if !self.commit() {
            return;
        }
        self.mode = Some(Mode::Find);
        self.req = 0;
        (self.request, self.omit) = self.requests[0].clone();
        self.fire(fmp12::triggers::event::MODE_ENTER);
    }

    /// Records menu (Browse mode commands).
    pub fn records_menu(&mut self, ui: &mut egui::Ui) {
        let browse = self.doc.is_some() && self.mode() == Mode::Browse;
        let item = |ui: &mut egui::Ui, label: &'static str, key: &str, on: bool| {
            ui.add_enabled(on, egui::Button::new(t(label)).shortcut_text(key))
                .clicked()
        };
        if item(ui, "New Record", "Ctrl+N", browse) {
            self.new_record();
            ui.close();
        }
        if item(ui, "Duplicate Record", "Ctrl+D", browse) {
            self.duplicate_record();
            ui.close();
        }
        if item(ui, "Delete Record…", "Ctrl+E", browse) {
            self.m8.confirm = Some(Confirm::Delete);
            ui.close();
        }
        if item(ui, "Delete All Records…", "", browse) {
            self.m8.confirm = Some(Confirm::DeleteAll);
            ui.close();
        }
        ui.separator();
        if item(ui, "Show All Records", "Ctrl+J", browse) {
            self.show_all();
            ui.close();
        }
        if item(ui, "Show Omitted Only", "", browse) {
            self.show_omitted();
            ui.close();
        }
        if item(ui, "Omit Record", "Ctrl+T", browse) {
            self.omit_record();
            ui.close();
        }
        if item(ui, "Omit Multiple…", "Ctrl+Shift+T", browse) {
            self.m8.omit = Some("1".into());
            ui.close();
        }
        ui.separator();
        if item(ui, "Modify Last Find", "Ctrl+R", browse) {
            self.modify_last_find();
            ui.close();
        }
        ui.separator();
        if item(ui, "Replace Field Contents…", "Ctrl+=", browse) {
            self.open_replace_contents();
            ui.close();
        }
        self.records_menu_more(ui);
    }

    /// File menu entries: Import Records › File…, Export Records….
    pub fn file_menu_items(&mut self, ui: &mut egui::Ui) {
        let on = self.doc.is_some() && self.mode() == Mode::Browse;
        ui.add_enabled_ui(on, |ui| {
            ui.menu_button(t("Import Records"), |ui| {
                if ui.button(t("File…")).clicked() {
                    ui.close();
                    self.pick_import_file(ui.ctx());
                }
            });
        });
        if ui
            .add_enabled(on, egui::Button::new(t("Export Records…")))
            .clicked()
        {
            ui.close();
            self.open_export();
        }
    }

    /// Edit menu entry outside Layout mode.
    pub fn edit_menu_items(&mut self, ui: &mut egui::Ui) {
        let on = self.doc.is_some() && self.mode() == Mode::Browse;
        let b = egui::Button::new(t("Find/Replace…")).shortcut_text("Ctrl+Shift+F");
        if ui.add_enabled(on, b).clicked() {
            self.open_find_replace();
            ui.close();
        }
    }

    fn open_find_replace(&mut self) {
        if self.m8.find.is_none() {
            self.m8.find = Some(FindDialog {
                q: FindReplace::default(),
                dir: Direction::All,
                current_only: false,
                current_field: false,
                last: None,
                status: String::new(),
            });
        }
    }

    fn open_replace_contents(&mut self) {
        if !self.commit() {
            return;
        }
        let field = self
            .focused_field()
            .or_else(|| self.layout_fields().first().copied());
        let value = match (self.table(), self.current_record(), field, &self.doc) {
            (Some(t), Some(r), Some(f), Some(d)) => {
                d.db.table(t)
                    .and_then(|x| x.records.get(r)?.values.get(&f).cloned())
                    .unwrap_or_default()
            }
            _ => String::new(),
        };
        self.m8.replace = Some(ReplaceContents {
            field,
            kind: 0,
            value,
            initial: "1".into(),
            increment: "1".into(),
            update: false,
            calc: String::new(),
            auto_enter: true,
        });
    }

    pub(crate) fn open_export(&mut self) {
        if !self.commit() {
            return;
        }
        // The privilege set's "Allow exporting".
        if self.doc.as_ref().is_some_and(|d| !d.db.can_export()) {
            let msg = crate::security_ui::denied().to_string();
            self.messages.push((String::new(), msg));
            return;
        }
        let Some(d) = self.doc.as_ref() else { return };
        let Some(l) = d.layouts.get(self.layout) else {
            return;
        };
        let to = (l.table_occurrence & 0xFFF) as u32;
        let group = vec![false; self.last_sort.len()];
        self.m8.export = Some(Export {
            format: Format::Csv,
            fields: Vec::new(),
            pick: None,
            to,
            browse: 0,
            formatted: false,
            group,
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn pick_import_file(&mut self, _ctx: &egui::Context) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter(
                "BaseCraft",
                &["csv", "tab", "txt", "tsv", "mer", "xlsx", "fmp12"],
            )
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&p) {
            Ok(b) => {
                let name = p
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                self.start_import(name, b);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn pick_import_file(&mut self, ctx: &egui::Context) {
        let slot = self.m8.picked.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new()
                .add_filter(
                    "BaseCraft",
                    &["csv", "tab", "txt", "tsv", "mer", "xlsx", "fmp12"],
                )
                .pick_file()
                .await
            {
                let data = h.read().await;
                *slot.borrow_mut() = Some((h.file_name(), data));
                ctx.request_repaint();
            }
        });
    }

    /// Reads an import source and opens the field mapping dialog.
    pub fn start_import(&mut self, name: String, bytes: Vec<u8>) {
        let Some(d) = self.doc.as_ref() else { return };
        let fmp = name.to_ascii_lowercase().ends_with(".fmp12");
        let (format, tables, rows) = if fmp {
            match engine::import::fmp12_source(&bytes, &d.db.locale) {
                Ok(tables) => {
                    let rows = tables.first().map(|x| x.1.clone()).unwrap_or_default();
                    (Format::Tab, tables, rows)
                }
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        } else {
            let format = Format::from_name(&name).unwrap_or(Format::Csv);
            match engine::import::read_rows(format, &bytes) {
                Ok(rows) => (format, Vec::new(), rows),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        };
        let header = fmp || matches!(format, Format::Merge);
        let mut imp = Import {
            name,
            format,
            tables,
            src_table: 0,
            rows,
            header,
            targets: Vec::new(),
            arrows: Vec::new(),
            action: Action::Add,
            add_remaining: false,
            auto_enter: true,
            sample: 0,
        };
        self.arrange(&mut imp, !header);
        self.m8.import = Some(imp);
    }

    /// Importable fields of the current table (normal fields), in creation order.
    fn importable(&self) -> Vec<(u32, String)> {
        let (Some(d), Some(t)) = (self.doc.as_ref(), self.table()) else {
            return Vec::new();
        };
        d.db.table(t).map_or(Vec::new(), |x| {
            x.fields
                .iter()
                .filter(|f| f.kind == FieldKind::Normal)
                .map(|f| (f.id, f.name.clone()))
                .collect()
        })
    }

    /// Arrange by matching names (or by creation order).
    fn arrange(&self, imp: &mut Import, creation: bool) {
        let fields = self.importable();
        let n = imp.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let names = imp.rows.first().cloned().unwrap_or_default();
        imp.targets = (0..n)
            .map(|i| {
                if creation {
                    return fields.get(i).map(|f| f.0);
                }
                let n = names.get(i).map_or("", |s| s.trim());
                fields
                    .iter()
                    .find(|f| f.1.eq_ignore_ascii_case(n))
                    .map(|f| f.0)
            })
            .collect();
        imp.arrows = imp
            .targets
            .iter()
            .map(|x| {
                if x.is_some() {
                    Arrow::Import
                } else {
                    Arrow::Skip
                }
            })
            .collect();
    }

    fn run_import(&mut self, imp: &Import) {
        let Some(tb) = self.table() else { return };
        let fields = self.importable();
        let name = |f: Option<u32>| {
            f.and_then(|f| fields.iter().find(|x| x.0 == f))
                .map_or(String::new(), |x| x.1.clone())
        };
        let names: Vec<String> = imp
            .targets
            .iter()
            .zip(&imp.arrows)
            .map(|(f, a)| match a {
                Arrow::Skip => String::new(),
                _ => name(*f),
            })
            .collect();
        let match_fields: Vec<String> = imp
            .targets
            .iter()
            .zip(&imp.arrows)
            .filter(|(f, a)| **a == Arrow::Match && f.is_some())
            .map(|(f, _)| name(*f))
            .collect();
        let mode = match imp.action {
            Action::Add => engine::import::Mode::Add,
            Action::Replace => engine::import::Mode::Replace {
                add_remaining: imp.add_remaining,
            },
            Action::Update => engine::import::Mode::Update {
                match_fields,
                add_remaining: imp.add_remaining,
            },
        };
        let opts = engine::import::Options {
            format: imp.format,
            first_row_names: imp.header,
            mapping: engine::import::Mapping::ByOrder(names),
            mode,
            auto_enter: imp.auto_enter,
        };
        let found = self.found(tb).clone();
        let Some(d) = self.doc.as_mut() else { return };
        match engine::import::import(&mut d.db, tb, &found, imp.rows.clone(), &opts) {
            Ok(rep) => {
                let n = rep.added.len() + rep.updated.len();
                self.found.insert(tb, rep.found.clone());
                self.sorted.remove(&tb);
                self.current.insert(tb, 0);
                self.dirty = true;
                self.messages.push((
                    t("Import Summary").to_string(),
                    format!(
                        "{} {n}\n{} {}",
                        t("Total records added / updated:"),
                        t("Total records skipped due to errors:"),
                        rep.skipped
                    ),
                ));
            }
            Err(e) => self.messages.push((t("Import Records").to_string(), e)),
        }
    }

    fn write_export(&mut self, ctx: &egui::Context, ex: &Export) {
        if let Some((name, bytes)) = self.export_bytes(ex) {
            save_bytes(ctx, name, extension(ex.format), bytes, &mut self.error);
        }
    }

    /// The exported file: its default name and contents.
    fn export_bytes(&mut self, ex: &Export) -> Option<(String, Vec<u8>)> {
        let t = self.table()?;
        let found = self.found(t).clone();
        let d = self.doc.as_ref()?;
        let columns: Vec<engine::export::Column> = ex
            .fields
            .iter()
            .filter_map(|&(to, f)| {
                let tbl = d.db.table_of(to)?;
                let fd = tbl.field(f)?;
                let name = if to == ex.to {
                    fd.name.clone()
                } else {
                    let o = d.db.occurrences.iter().find(|o| o.id == to)?;
                    format!("{}::{}", o.name, fd.name)
                };
                Some(engine::export::Column {
                    to,
                    table: tbl.id,
                    field: f,
                    name,
                    data_type: fd.data_type,
                })
            })
            .collect();
        let group: Vec<u32> = self
            .last_sort
            .iter()
            .zip(&ex.group)
            .filter(|(_, g)| **g)
            .map(|((f, _), _)| *f)
            .collect();
        let file_name = format!("{}.fmp12", d.name);
        let bytes = engine::export::Export {
            db: &d.db,
            to: ex.to,
            found: &found,
            columns: &columns,
            formatted: ex.formatted,
            file_name: &file_name,
            group_by: &group,
        }
        .write(ex.format);
        Some((format!("{}.{}", d.name, extension(ex.format)), bytes))
    }

    /// Draws the open dialogs and handles their shortcuts. Called once per frame.
    pub fn records_dialogs(&mut self, ctx: &egui::Context) {
        let picked = self.m8.picked.borrow_mut().take();
        if let Some((name, bytes)) = picked {
            self.start_import(name, bytes);
        }
        self.record_keys(ctx);
        self.confirm_ui(ctx);
        self.omit_ui(ctx);
        self.find_replace_ui(ctx);
        self.replace_contents_ui(ctx);
        self.import_ui(ctx);
        self.export_ui(ctx);
    }

    fn record_keys(&mut self, ctx: &egui::Context) {
        if self.doc.is_none() || self.mode() != Mode::Browse || self.active.is_some() {
            return;
        }
        use egui::{Key, Modifiers as M};
        let cs = M::COMMAND | M::SHIFT;
        let key = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if key(cs, Key::F) {
            self.open_find_replace();
        } else if key(cs, Key::T) {
            self.m8.omit = Some("1".into());
        } else if key(M::COMMAND, Key::T) {
            self.omit_record();
        } else if key(M::COMMAND, Key::D) {
            self.duplicate_record();
        } else if key(M::COMMAND, Key::E) {
            self.m8.confirm = Some(Confirm::Delete);
        } else if key(M::COMMAND, Key::J) {
            self.show_all();
        } else if key(M::COMMAND, Key::R) {
            self.modify_last_find();
        } else if key(M::COMMAND, Key::Equals) {
            self.open_replace_contents();
        } else if key(M::COMMAND, Key::N) {
            self.new_record();
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = self.m8.confirm else { return };
        let n = self.table().map_or(0, |t| self.found(t).len());
        let (msg, ok) = match c {
            Confirm::Delete => (
                t("Permanently delete this ENTIRE record?").to_string(),
                t("Delete"),
            ),
            Confirm::DeleteAll => (
                if i18n::french() {
                    format!(
                        "Supprimer définitivement les {n} enregistrement(s) de l'ensemble trouvé ?"
                    )
                } else {
                    format!("Permanently delete ALL {n} record(s) in this found set?")
                },
                t("Delete All"),
            ),
        };
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("m8_confirm")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(msg);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
                if ui.button(ok).clicked() {
                    done = Some(true);
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        let Some(yes) = done else { return };
        self.m8.confirm = None;
        if !yes {
            return;
        }
        let Some(tb) = self.table() else { return };
        match c {
            Confirm::Delete => {
                if let Some(r) = self.current_record() {
                    self.delete_records(&[r]);
                }
            }
            Confirm::DeleteAll => {
                let all = self.found(tb).clone();
                self.delete_records(&all);
            }
        }
        self.fire(fmp12::triggers::event::RECORD_LOAD);
    }

    fn omit_ui(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.m8.omit.take() else {
            return;
        };
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("m8_omit")).show(ctx, |ui| {
            ui.heading(t("Omit Multiple Records"));
            ui.horizontal(|ui| {
                let l = ui.label(t("Number of records to omit:"));
                ui.add(egui::TextEdit::singleline(&mut s).desired_width(60.0))
                    .labelled_by(l.id);
            });
            ui.horizontal(|ui| {
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
                if ui.button(t("Omit")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
            });
        });
        match done {
            None => self.m8.omit = Some(s),
            Some(true) => self.omit_multiple(s.trim().parse().unwrap_or(0)),
            Some(false) => {}
        }
    }

    fn find_replace_ui(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.m8.find.take() else {
            return;
        };
        let mut open = true;
        let mut act = None;
        egui::Window::new(t("Find/Replace"))
            .id(egui::Id::NULL.with("m8_find"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos([220.0, 200.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        egui::Grid::new("fr_grid").num_columns(2).show(ui, |ui| {
                            let l = ui.label(t("Find what:"));
                            ui.add(egui::TextEdit::singleline(&mut st.q.find).desired_width(300.0))
                                .labelled_by(l.id);
                            ui.end_row();
                            let l = ui.label(t("Replace with:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut st.q.replace).desired_width(300.0),
                            )
                            .labelled_by(l.id);
                            ui.end_row();
                            ui.label(t("Direction:"));
                            egui::ComboBox::from_id_salt("fr_dir")
                                .selected_text(match st.dir {
                                    Direction::Forward => t("Forward"),
                                    Direction::Backward => t("Backward"),
                                    Direction::All => t("All"),
                                })
                                .show_ui(ui, |ui| {
                                    for (d, n) in [
                                        (Direction::Forward, "Forward"),
                                        (Direction::Backward, "Backward"),
                                        (Direction::All, "All"),
                                    ] {
                                        ui.selectable_value(&mut st.dir, d, t(n));
                                    }
                                });
                            ui.end_row();
                        });
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.add_space(18.0);
                                ui.checkbox(&mut st.q.match_case, t("Match case"));
                                ui.checkbox(&mut st.q.whole_word, t("Match whole word"));
                            });
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.label(t("Search across:"));
                                ui.radio_value(&mut st.current_only, false, t("All records"));
                                ui.radio_value(&mut st.current_only, true, t("Current record"));
                            });
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.label(t("Search within:"));
                                ui.radio_value(&mut st.current_field, false, t("All fields"));
                                ui.radio_value(&mut st.current_field, true, t("Current field"));
                            });
                        });
                    });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        let w = egui::vec2(150.0, 0.0);
                        for (label, a) in [
                            ("Find Next", 0),
                            ("Replace & Find", 1),
                            ("Replace", 2),
                            ("Replace All", 3),
                        ] {
                            if ui.add(egui::Button::new(t(label)).min_size(w)).clicked() {
                                act = Some(a);
                            }
                        }
                        ui.add_space(20.0);
                        if ui.add(egui::Button::new(t("Close")).min_size(w)).clicked() {
                            act = Some(9);
                        }
                    });
                });
                if !st.status.is_empty() {
                    ui.label(&st.status);
                }
                // Enter = Find Next (the default button).
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    act = act.or(Some(0));
                }
            });
        match act {
            Some(9) => return,
            Some(a) => self.find_replace_do(&mut st, a),
            None => {}
        }
        if open {
            self.m8.find = Some(st);
        }
    }

    /// Find Next (0), Replace & Find (1), Replace (2), Replace All (3).
    fn find_replace_do(&mut self, st: &mut FindDialog, act: u8) {
        let Some(t0) = self.table() else { return };
        let mut fields = self.layout_fields();
        if st.current_field {
            fields = self.focused_field().into_iter().collect();
        }
        let pos = self.current.get(&t0).copied().unwrap_or(0);
        let found = self.found(t0).clone();
        let records: Vec<usize> = if st.current_only {
            found.get(pos).copied().into_iter().collect()
        } else {
            found.clone()
        };
        let Some(d) = self.doc.as_mut() else { return };
        let db = &mut d.db;
        st.status.clear();
        if act == 3 {
            let n = engine::replace::replace_all(db, t0, &records, &fields, &st.q);
            self.dirty |= n > 0;
            st.status = if i18n::french() {
                format!("{n} occurrence(s) remplacée(s).")
            } else {
                format!("{n} occurrence(s) replaced.")
            };
            st.last = None;
            return;
        }
        if matches!(act, 1 | 2)
            && let Some(h) = &st.last
            && engine::replace::replace_hit(db, t0, &records, &st.q, h)
        {
            self.dirty = true;
            // The next search starts after the replacement.
            let end = h.range.start + st.q.replace.len();
            st.last = Some(Hit {
                range: h.range.start..end,
                ..h.clone()
            });
            if act == 2 {
                return;
            }
        }
        // Without a previous hit, a forward search starts at the current
        // record (just after the record before it).
        let start = st.last.clone().or_else(|| {
            (!st.current_only && pos > 0 && st.dir != Direction::Backward).then(|| Hit {
                pos: pos - 1,
                field: u32::MAX,
                range: usize::MAX..usize::MAX,
            })
        });
        let next =
            engine::replace::find_next(db, t0, &records, &fields, &st.q, start.as_ref(), st.dir);
        match next {
            Some(h) => {
                if !st.current_only {
                    self.current.insert(t0, h.pos);
                }
                let name = db
                    .table(t0)
                    .and_then(|x| x.field(h.field))
                    .map_or(String::new(), |f| f.name.clone());
                st.status = format!("{} {} — {name}", t("Record"), h.pos + 1);
                st.last = Some(h);
            }
            None => {
                st.status = t("No match found.").to_string();
                st.last = None;
            }
        }
    }

    fn replace_contents_ui(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.m8.replace.take() else {
            return;
        };
        let fields = self.importable();
        let n = self.table().map_or(0, |t| self.found(t).len());
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("m8_replace")).show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.strong(t("Replace Field Contents"));
            let name = fields
                .iter()
                .find(|f| Some(f.0) == st.field)
                .map_or(String::new(), |f| f.1.clone());
            // Without a field that has the focus, the field is picked here.
            if self.m8.focus.is_none() {
                ui.horizontal(|ui| {
                    ui.label(t("Field:"));
                    egui::ComboBox::from_id_salt("rfc_field")
                        .selected_text(&name)
                        .show_ui(ui, |ui| {
                            for (id, n) in &fields {
                                ui.selectable_value(&mut st.field, Some(*id), n);
                            }
                        });
                });
            }
            ui.label(if i18n::french() {
                format!(
                    "Dans les {n} enregistrements en cours d'utilisation, remplacer définitivement le contenu de la rubrique \"{name}\" ?"
                )
            } else {
                format!(
                    "Permanently replace the contents of the field \"{name}\" in the {n} records of the current found set?"
                )
            });
            ui.add_space(6.0);
            let current = if i18n::french() {
                format!("Remplacer par : \"{}\"", st.value)
            } else {
                format!("Replace with: \"{}\"", st.value)
            };
            ui.radio_value(&mut st.kind, 0, current);
            ui.radio_value(&mut st.kind, 1, t("Replace with serial numbers:"));
            ui.add_enabled_ui(st.kind == 1, |ui| {
                egui::Grid::new("rfc_serial").show(ui, |ui| {
                    ui.label("      ");
                    let l = ui.label(t("Initial value:"));
                    ui.add(egui::TextEdit::singleline(&mut st.initial).desired_width(200.0))
                        .labelled_by(l.id);
                    ui.end_row();
                    ui.label("      ");
                    let l = ui.label(t("Increment by:"));
                    ui.add(egui::TextEdit::singleline(&mut st.increment).desired_width(200.0))
                        .labelled_by(l.id);
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.checkbox(&mut st.update, t("Update serial number in Entry Options"));
                });
            });
            ui.radio_value(&mut st.kind, 2, t("Replace with calculated result:"));
            ui.add_enabled(
                st.kind == 2,
                egui::TextEdit::multiline(&mut st.calc)
                    .desired_rows(2)
                    .code_editor(),
            );
            ui.separator();
            ui.checkbox(&mut st.auto_enter, t("Perform auto-enter options while replacing"));
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
                if ui
                    .add_enabled(st.field.is_some(), egui::Button::new(t("Replace")))
                    .clicked()
                {
                    done = Some(true);
                }
            });
        });
        match done {
            None => self.m8.replace = Some(st),
            Some(false) => {}
            Some(true) => {
                let (Some(tb), Some(field)) = (self.table(), st.field) else {
                    return;
                };
                let to = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.layouts.get(self.layout))
                    .map_or(0, |l| (l.table_occurrence & 0xFFF) as u32);
                let with = match st.kind {
                    0 => ReplaceWith::Value(st.value.clone()),
                    1 => ReplaceWith::Serial {
                        initial: st.initial.clone(),
                        increment: st.increment.trim().parse().unwrap_or(1),
                        update_entry: st.update,
                    },
                    _ => ReplaceWith::Calc {
                        source: st.calc.clone(),
                        to,
                    },
                };
                let found = self.found(tb).clone();
                if let Some(d) = self.doc.as_mut() {
                    match engine::replace::replace_field_contents(
                        &mut d.db,
                        tb,
                        &found,
                        field,
                        &with,
                        st.auto_enter,
                    ) {
                        Ok(_) => self.dirty = true,
                        Err(e) => self.messages.push((String::new(), e)),
                    }
                }
            }
        }
    }

    fn import_ui(&mut self, ctx: &egui::Context) {
        let Some(mut imp) = self.m8.import.take() else {
            return;
        };
        let fields = self.importable();
        let target_name = self
            .table()
            .and_then(|tb| self.doc.as_ref()?.db.table(tb))
            .map_or(String::new(), |x| x.name.clone());
        let mut done = None;
        let mut arrange = None;
        let green = egui::Color32::from_rgb(110, 190, 80);
        egui::Modal::new(egui::Id::NULL.with("m8_import")).show(ctx, |ui| {
            ui.set_width(720.0);
            ui.strong(t("Import Field Mapping"));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    ui.strong(t("Set the import order of the target fields."));
                    ui.horizontal(|ui| {
                        let total = imp.rows.len();
                        if ui.button("◀").clicked() && imp.sample > 0 {
                            imp.sample -= 1;
                        }
                        if ui.button("▶").clicked() && imp.sample + 1 < total {
                            imp.sample += 1;
                        }
                        ui.label(format!("{} {} {total}", imp.sample + 1, t("of")));
                    });
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(t("SOURCE"));
                            ui.label(&imp.name);
                            if !imp.tables.is_empty() {
                                let cur = imp.tables.get(imp.src_table).map_or("", |x| x.0.as_str());
                                egui::ComboBox::from_id_salt("imp_src")
                                    .selected_text(cur)
                                    .show_ui(ui, |ui| {
                                        for i in 0..imp.tables.len() {
                                            if ui
                                                .selectable_label(i == imp.src_table, &imp.tables[i].0)
                                                .clicked()
                                            {
                                                imp.src_table = i;
                                                imp.rows = imp.tables[i].1.clone();
                                                arrange = Some(false);
                                            }
                                        }
                                    });
                            }
                        });
                        // The action, above the arrow; clicking it shows the choices.
                        let action = match imp.action {
                            Action::Add => t("Add"),
                            Action::Update => t("Update"),
                            Action::Replace => t("Replace"),
                        };
                        ui.vertical(|ui| {
                            ui.menu_button(action, |ui| {
                                ui.radio_value(&mut imp.action, Action::Add, t("Add"));
                                ui.label(t("Add new records"));
                                ui.radio_value(&mut imp.action, Action::Update, t("Update"));
                                ui.label(t(
                                    "Update matching records in the found set when match fields are equal",
                                ));
                                ui.radio_value(&mut imp.action, Action::Replace, t("Replace"));
                                ui.label(t("Replace the found set's data in record order"));
                                ui.separator();
                                ui.add_enabled(
                                    imp.action != Action::Add,
                                    egui::Checkbox::new(
                                        &mut imp.add_remaining,
                                        t("Add remaining data as new records"),
                                    ),
                                );
                            });
                            ui.label("———▶");
                        });
                        ui.vertical(|ui| {
                            ui.strong(t("TARGET"));
                            ui.label(format!("{} (\"{target_name}\")", t("Current Table")));
                        });
                    });
                });
            });
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                egui::Grid::new("imp_grid")
                    .striped(true)
                    .min_col_width(150.0)
                    .show(ui, |ui| {
                        ui.label("");
                        ui.strong(t("Source fields"));
                        ui.strong(t("Mapping"));
                        ui.horizontal(|ui| {
                            ui.strong(t("Target fields"));
                            ui.menu_button("▼", |ui| {
                                if ui.button(t("Matching names")).clicked() {
                                    arrange = Some(false);
                                    ui.close();
                                }
                                if ui.button(t("Creation order")).clicked() {
                                    arrange = Some(true);
                                    ui.close();
                                }
                            });
                        });
                        ui.end_row();
                        let names = imp.rows.first().cloned().unwrap_or_default();
                        let sample = imp.rows.get(imp.sample).cloned().unwrap_or_default();
                        for i in 0..imp.targets.len() {
                            let name = if imp.header {
                                names.get(i).map_or("", |s| s.as_str())
                            } else {
                                ""
                            };
                            ui.weak(name);
                            let v = sample.get(i).map_or("", |s| s.as_str());
                            let v: String = v.chars().take(40).collect();
                            ui.label(v.replace(['\r', '\n'], " "));
                            let (text, fill) = match imp.arrows[i] {
                                Arrow::Import => (t("Import"), green),
                                Arrow::Match => ("=", green),
                                Arrow::Skip => (t("Don't import"), egui::Color32::from_gray(225)),
                            };
                            // Clicking cycles Import → (match) → Don't import.
                            let b = egui::Button::new(text).fill(fill).min_size(egui::vec2(140.0, 0.0));
                            if ui.add(b).clicked() {
                                imp.arrows[i] = match (imp.arrows[i], imp.action) {
                                    (Arrow::Import, Action::Update) => Arrow::Match,
                                    (Arrow::Import, _) | (Arrow::Match, _) => Arrow::Skip,
                                    (Arrow::Skip, _) => Arrow::Import,
                                };
                            }
                            let name = fields
                                .iter()
                                .find(|f| Some(f.0) == imp.targets[i])
                                .map_or("—", |f| f.1.as_str());
                            egui::ComboBox::from_id_salt(("imp_t", i))
                                .selected_text(egui::RichText::new(name).strong())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut imp.targets[i], None, "—");
                                    for (id, n) in &fields {
                                        ui.selectable_value(&mut imp.targets[i], Some(*id), n);
                                    }
                                });
                            ui.end_row();
                        }
                    });
            });
            ui.checkbox(
                &mut imp.header,
                t("Don't import first record (contains field names)"),
            );
            ui.checkbox(
                &mut imp.auto_enter,
                t("Perform auto-enter options while importing"),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
                if ui.button(t("Import")).clicked() {
                    done = Some(true);
                }
            });
        });
        if imp.action != Action::Update {
            for a in imp.arrows.iter_mut().filter(|a| **a == Arrow::Match) {
                *a = Arrow::Import;
            }
        }
        if let Some(creation) = arrange {
            self.arrange(&mut imp, creation);
        }
        match done {
            None => self.m8.import = Some(imp),
            Some(false) => {}
            Some(true) => self.run_import(&imp),
        }
    }

    fn export_ui(&mut self, ctx: &egui::Context) {
        let Some(mut ex) = self.m8.export.take() else {
            return;
        };
        let Some(d) = self.doc.as_ref() else { return };
        let label = |to: u32, f: u32| -> String {
            let Some(tbl) = d.db.table_of(to) else {
                return String::new();
            };
            let name = tbl.field(f).map_or("", |x| x.name.as_str());
            if to == ex.to {
                name.to_string()
            } else {
                let o = d.db.occurrences.iter().find(|o| o.id == to);
                format!("{}::{name}", o.map_or("", |o| o.name.as_str()))
            }
        };
        // Occurrences: the layout's, then those related to it.
        let mut tos = vec![ex.to];
        for r in &d.db.relationships {
            for (a, b) in [(r.left, r.right), (r.right, r.left)] {
                if a == ex.to && !tos.contains(&b) {
                    tos.push(b);
                }
            }
        }
        let layout_name = d.layouts.get(self.layout).map_or("", |l| l.name.as_str());
        let occ_name = |to: u32| {
            if to == ex.to {
                return format!("{} (\"{layout_name}\")", t("Current Layout"));
            }
            d.db.occurrences
                .iter()
                .find(|o| o.id == to)
                .map_or(String::new(), |o| o.name.clone())
        };
        let browse_to = ex.to_browse();
        let available: Vec<u32> = d.db.table_of(browse_to).map_or(Vec::new(), |t| {
            t.fields
                .iter()
                .filter(|f| f.data_type != fmp12::schema::DataType::Container)
                .map(|f| f.id)
                .collect()
        });
        let sort_names: Vec<String> = self
            .last_sort
            .iter()
            .map(|(f, _)| label(ex.to, *f))
            .collect();
        let mut done = None;
        egui::Modal::new(egui::Id::NULL.with("m8_export")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.strong(t("Specify Field Order for Export"));
            ui.label(t("Select and move fields to export. To group data, select the fields to group by before moving other fields."));
            ui.columns(3, |c| {
                egui::ComboBox::from_id_salt("exp_to")
                    .selected_text(occ_name(browse_to))
                    .width(190.0)
                    .show_ui(&mut c[0], |ui| {
                        for &to in &tos {
                            ui.selectable_value(&mut ex.browse, to, occ_name(to));
                        }
                    });
                egui::Frame::group(c[0].style()).show(&mut c[0], |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("exp_left")
                        .min_scrolled_height(220.0)
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.set_min_width(170.0);
                            for &f in &available {
                                let k = (browse_to, f);
                                let r = ui.selectable_label(ex.pick == Some(k), label(browse_to, f));
                                if r.clicked() {
                                    ex.pick = Some(k);
                                }
                                if r.double_clicked() && !ex.fields.contains(&k) {
                                    ex.fields.push(k);
                                }
                            }
                        });
                });
                c[1].vertical_centered(|ui| {
                    ui.add_space(70.0);
                    let w = egui::vec2(100.0, 0.0);
                    let can_move = ex.pick.is_some_and(|k| !ex.fields.contains(&k));
                    if ui
                        .add_enabled(can_move, egui::Button::new(t("Move")).min_size(w))
                        .clicked()
                        && let Some(k) = ex.pick
                    {
                        ex.fields.push(k);
                    }
                    if ui.add(egui::Button::new(t("Move All")).min_size(w)).clicked() {
                        for &f in &available {
                            if !ex.fields.contains(&(browse_to, f)) {
                                ex.fields.push((browse_to, f));
                            }
                        }
                    }
                    if ui
                        .add_enabled(
                            !ex.fields.is_empty(),
                            egui::Button::new(t("Clear All")).min_size(w),
                        )
                        .clicked()
                    {
                        ex.fields.clear();
                    }
                });
                c[2].label(t("Group by"));
                egui::Frame::group(c[2].style()).show(&mut c[2], |ui| {
                    ui.set_min_size(egui::vec2(170.0, 50.0));
                    if sort_names.is_empty() {
                        ui.strong(t("(Unsorted)"));
                    }
                    for (i, n) in sort_names.iter().enumerate() {
                        if let Some(g) = ex.group.get_mut(i) {
                            ui.checkbox(g, n);
                        }
                    }
                });
                c[2].label(t("Field export order"));
                egui::Frame::group(c[2].style()).show(&mut c[2], |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("exp_right")
                        .min_scrolled_height(130.0)
                        .max_height(130.0)
                        .show(ui, |ui| {
                            ui.set_min_width(170.0);
                            let mut swap = None;
                            let mut remove = None;
                            for (i, &(to, f)) in ex.fields.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui.small_button("↑").clicked() && i > 0 {
                                        swap = Some((i - 1, i));
                                    }
                                    if ui.small_button("↓").clicked() && i + 1 < ex.fields.len() {
                                        swap = Some((i, i + 1));
                                    }
                                    // Double-click takes a field out, as in FileMaker.
                                    if ui.selectable_label(false, label(to, f)).double_clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some((a, b)) = swap {
                                ex.fields.swap(a, b);
                            }
                            if let Some(i) = remove {
                                ex.fields.remove(i);
                            }
                        });
                });
            });
            ui.horizontal(|ui| {
                ui.label(t("Type:"));
                egui::ComboBox::from_id_salt("exp_fmt")
                    .selected_text(format_name(ex.format))
                    .show_ui(ui, |ui| {
                        for f in [
                            Format::Tab,
                            Format::Csv,
                            Format::Merge,
                            Format::Html,
                            Format::Xml,
                            Format::Xlsx,
                        ] {
                            ui.selectable_value(&mut ex.format, f, format_name(f));
                        }
                    });
            });
            ui.checkbox(&mut ex.formatted, t("Apply current layout's data formatting"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(t("Cancel")).clicked() {
                    done = Some(false);
                }
                if ui
                    .add_enabled(!ex.fields.is_empty(), egui::Button::new(t("Export")))
                    .clicked()
                {
                    done = Some(true);
                }
            });
        });
        match done {
            None => self.m8.export = Some(ex),
            Some(false) => {}
            Some(true) => self.write_export(ctx, &ex),
        }
    }
}

#[cfg(test)]
impl App {
    /// What Export would write for the open Export dialog (tests: the
    /// Export button opens a file dialog).
    pub fn export_now(&mut self) -> Option<Vec<u8>> {
        let ex = self.m8.export.take()?;
        self.export_bytes(&ex).map(|x| x.1)
    }

    pub fn close_find(&mut self) {
        self.m8.find = None;
    }

    pub fn dialogs_open(&self) -> bool {
        let m = &self.m8;
        m.find.is_some()
            || m.replace.is_some()
            || m.import.is_some()
            || m.export.is_some()
            || m.omit.is_some()
            || m.confirm.is_some()
    }
}

impl Export {
    /// Occurrence whose fields the left list shows (the layout's by default).
    fn to_browse(&self) -> u32 {
        if self.browse == 0 {
            self.to
        } else {
            self.browse
        }
    }
}

/// Saves `bytes`: a save dialog on desktop, a download in the browser.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_bytes(
    _ctx: &egui::Context,
    name: String,
    ext: &str,
    bytes: Vec<u8>,
    error: &mut Option<String>,
) {
    let Some(p) = rfd::FileDialog::new()
        .add_filter(ext, &[ext])
        .set_file_name(&name)
        .save_file()
    else {
        return;
    };
    if let Err(e) = std::fs::write(p, bytes) {
        *error = Some(e.to_string());
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn save_bytes(
    _ctx: &egui::Context,
    name: String,
    _ext: &str,
    bytes: Vec<u8>,
    _error: &mut Option<String>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Some(h) = rfd::AsyncFileDialog::new()
            .set_file_name(&name)
            .save_file()
            .await
        {
            let _ = h.write(&bytes).await;
        }
    });
}
