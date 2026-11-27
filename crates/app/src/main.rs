//! OpenMaker desktop application (egui).
//!
//! Browse and Find modes over a `.fmp12` file: layouts drawn at their real
//! coordinates, editable fields, new/delete record, find requests, sort.
//! Changes live in memory (saving to `.fmp12` is milestone M7).

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use engine::{Database, Order, Request};
use fmp12::layouts::{Bounds, Layout};

mod i18n;
use i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Find,
}

struct Doc {
    path: PathBuf,
    db: Database,
    /// Layouts that have a table occurrence (folders excluded).
    layouts: Vec<Layout>,
}

#[derive(Default)]
struct App {
    doc: Option<Doc>,
    error: Option<String>,
    layout: usize,
    /// Found set per base table (record indexes), and the current position.
    found: BTreeMap<u32, Vec<usize>>,
    current: BTreeMap<u32, usize>,
    sorted: BTreeMap<u32, bool>,
    mode: Option<Mode>,
    /// Find request being edited: field id → criterion.
    request: BTreeMap<u32, String>,
    sort_field: Option<u32>,
    dirty: bool,
}

impl App {
    fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Browse)
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("FileMaker", &["fmp12"])
            .pick_file()
        {
            self.open(p);
        }
    }

    fn open(&mut self, path: PathBuf) {
        match Database::open(&path) {
            Ok(db) => {
                let layouts = db
                    .layouts
                    .iter()
                    .filter(|l| l.table_occurrence != 0)
                    .cloned()
                    .collect();
                *self = App {
                    doc: Some(Doc { path, db, layouts }),
                    ..Default::default()
                };
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Base table id of the current layout.
    fn table(&self) -> Option<u32> {
        let d = self.doc.as_ref()?;
        let l = d.layouts.get(self.layout)?;
        Some(d.db.table_of((l.table_occurrence & 0xFFF) as u32)?.id)
    }

    fn found(&mut self, t: u32) -> &mut Vec<usize> {
        let n = self
            .doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |t| t.records.len());
        self.found.entry(t).or_insert_with(|| (0..n).collect())
    }

    fn current_record(&mut self) -> Option<usize> {
        let t = self.table()?;
        let pos = *self.current.get(&t).unwrap_or(&0);
        self.found(t).get(pos).copied()
    }

    fn new_record(&mut self) {
        let Some(t) = self.table() else { return };
        let Some(d) = self.doc.as_mut() else { return };
        let Some(tbl) = d.db.tables.iter_mut().find(|x| x.id == t) else {
            return;
        };
        let r = tbl.new_record();
        let f = self.found(t);
        f.push(r);
        let n = f.len();
        self.current.insert(t, n - 1);
        self.dirty = true;
    }

    fn delete_record(&mut self) {
        let Some(t) = self.table() else { return };
        let pos = *self.current.get(&t).unwrap_or(&0);
        let Some(r) = self.found(t).get(pos).copied() else {
            return;
        };
        if let Some(tbl) = self
            .doc
            .as_mut()
            .and_then(|d| d.db.tables.iter_mut().find(|x| x.id == t))
        {
            tbl.records.remove(r);
        }
        // Re-index the found set after removal.
        let f = self.found(t);
        f.retain(|&x| x != r);
        for x in f.iter_mut() {
            if *x > r {
                *x -= 1;
            }
        }
        let n = f.len();
        self.current.insert(t, pos.min(n.saturating_sub(1)));
        self.dirty = true;
    }

    fn show_all(&mut self) {
        if let Some(t) = self.table() {
            self.found.remove(&t);
            self.sorted.remove(&t);
            self.current.insert(t, 0);
        }
    }

    fn perform_find(&mut self) {
        let (Some(t), Some(d)) = (self.table(), self.doc.as_ref()) else {
            return;
        };
        let req = Request {
            criteria: self
                .request
                .iter()
                .filter(|(_, v)| !v.trim().is_empty())
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            omit: false,
        };
        let found = engine::find(&d.db, t, &[req]);
        self.found.insert(t, found);
        self.current.insert(t, 0);
        self.mode = Some(Mode::Browse);
    }

    fn sort(&mut self, order: Order) {
        let (Some(t), Some(f)) = (self.table(), self.sort_field) else {
            return;
        };
        let mut found = self.found(t).clone();
        if let Some(d) = &self.doc {
            engine::sort(&d.db, t, &mut found, &[(f, order)]);
        }
        self.found.insert(t, found);
        self.sorted.insert(t, true);
        self.current.insert(t, 0);
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File"), |ui| {
                if ui.button(tr("Open…")).clicked() {
                    ui.close();
                    self.open_dialog();
                }
                if ui.button(tr("Close")).clicked() {
                    ui.close();
                    *self = App::default();
                }
                ui.separator();
                if ui.button(tr("Exit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr("Edit"), |ui| {
                ui.label(tr("Not implemented yet"));
            });
            ui.menu_button(tr("View"), |ui| {
                if ui.button(tr("Browse Mode")).clicked() {
                    self.mode = Some(Mode::Browse);
                    ui.close();
                }
                if ui.button(tr("Find Mode")).clicked() {
                    self.mode = Some(Mode::Find);
                    self.request.clear();
                    ui.close();
                }
            });
            ui.menu_button(tr("Insert"), |ui| {
                ui.label(tr("Not implemented yet"));
            });
            ui.menu_button(tr("Format"), |ui| {
                ui.label(tr("Not implemented yet"));
            });
            ui.menu_button(tr("Records"), |ui| {
                if ui.button(tr("New Record")).clicked() {
                    self.new_record();
                    ui.close();
                }
                if ui.button(tr("Delete Record…")).clicked() {
                    self.delete_record();
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("Show All Records")).clicked() {
                    self.show_all();
                    ui.close();
                }
            });
            for m in ["Scripts", "Tools", "Window", "Help"] {
                ui.menu_button(tr(m), |ui| {
                    ui.label(tr("Not implemented yet"));
                });
            }
        });
    }

    /// Status toolbar: navigation, counts, record commands.
    fn status_toolbar(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.table() else { return };
        let total = self
            .doc
            .as_ref()
            .and_then(|d| d.db.table(t))
            .map_or(0, |x| x.records.len());
        let found = self.found(t).len();
        let pos = *self.current.get(&t).unwrap_or(&0);
        ui.horizontal(|ui| {
            if ui.button("◀").clicked() && pos > 0 {
                self.current.insert(t, pos - 1);
            }
            if ui.button("▶").clicked() && pos + 1 < found {
                self.current.insert(t, pos + 1);
            }
            ui.label(if found == 0 {
                "0".to_string()
            } else {
                (pos + 1).to_string()
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label(format!("{found} / {total}"));
                let s = if self.sorted.get(&t).copied().unwrap_or(false) {
                    "Sorted"
                } else {
                    "Unsorted"
                };
                ui.label(format!("{} ({})", tr("Total"), tr(s)));
            });
            ui.separator();
            match self.mode() {
                Mode::Browse => {
                    if ui.button(tr("Show All")).clicked() {
                        self.show_all();
                    }
                    if ui.button(tr("New Record")).clicked() {
                        self.new_record();
                    }
                    if ui
                        .add_enabled(found > 0, egui::Button::new(tr("Delete Record")))
                        .clicked()
                    {
                        self.delete_record();
                    }
                    if ui.button(tr("Find")).clicked() {
                        self.mode = Some(Mode::Find);
                        self.request.clear();
                    }
                    let fields: Vec<(u32, String)> = self
                        .doc
                        .as_ref()
                        .and_then(|d| d.db.table(t))
                        .map(|x| x.fields.iter().map(|f| (f.id, f.name.clone())).collect())
                        .unwrap_or_default();
                    let label = self
                        .sort_field
                        .and_then(|id| fields.iter().find(|f| f.0 == id))
                        .map_or(tr("Sort"), |f| f.1.as_str());
                    egui::ComboBox::from_id_salt("sortfield")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, name) in &fields {
                                ui.selectable_value(&mut self.sort_field, Some(*id), name);
                            }
                        });
                    if ui.button("↑").clicked() {
                        self.sort(Order::Ascending);
                    }
                    if ui.button("↓").clicked() {
                        self.sort(Order::Descending);
                    }
                }
                Mode::Find => {
                    if ui.button(tr("Perform Find")).clicked() {
                        self.perform_find();
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.mode = Some(Mode::Browse);
                    }
                }
            }
            if self.dirty {
                ui.colored_label(
                    Color32::from_rgb(180, 90, 0),
                    tr("Unsaved changes (in memory)"),
                );
            }
        });
    }

    fn layout_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(tr("Layout:"));
            let current = doc.layouts.get(self.layout).map_or("", |l| l.name.as_str());
            egui::ComboBox::from_id_salt("layout")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, l) in doc.layouts.iter().enumerate() {
                        if ui.selectable_label(i == self.layout, &l.name).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            ui.label(tr("View As:"));
            ui.add_enabled(false, egui::Button::new(tr("Form")));
            ui.add_enabled(false, egui::Button::new(tr("List")));
            ui.add_enabled(false, egui::Button::new(tr("Table")));
            ui.add_enabled(false, egui::Button::new(tr("Preview")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled(false, egui::Button::new(tr("Edit Layout")));
            });
        });
        if let Some(i) = chosen {
            self.layout = i;
        }
    }

    /// Draws the current layout in form view, with editable fields.
    fn content(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.table() else { return };
        let record = self.current_record();
        let mode = self.mode();
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(layout) = doc.layouts.get(self.layout).cloned() else {
            return;
        };
        let height = layout
            .parts
            .iter()
            .map(|p| p.top + p.height)
            .fold(0.0, f64::max);
        let (resp, painter) = ui.allocate_painter(
            Vec2::new(layout.width as f32, height as f32),
            egui::Sense::hover(),
        );
        let origin = resp.rect.min;
        let at = |b: &Bounds| {
            Rect::from_min_max(
                origin + Vec2::new(b.left as f32, b.top as f32),
                origin + Vec2::new(b.right as f32, b.bottom as f32),
            )
        };
        let bg = if mode == Mode::Find {
            Color32::from_rgb(235, 240, 250)
        } else {
            Color32::from_gray(245)
        };
        painter.rect_filled(resp.rect, 0.0, bg);
        for p in &layout.parts {
            let y = origin.y + (p.top + p.height) as f32;
            painter.line_segment(
                [
                    Pos2::new(origin.x, y),
                    Pos2::new(origin.x + layout.width as f32, y),
                ],
                Stroke::new(0.5, Color32::from_gray(210)),
            );
        }
        let font = egui::FontId::proportional(13.0);
        let frame = Stroke::new(1.0, Color32::from_gray(200));
        for o in &layout.objects {
            let r = at(&o.bounds);
            match o.kind {
                1 => {
                    let Some((to, field, _)) = o.field else {
                        continue;
                    };
                    let ft = doc.db.table_of((to & 0xFFF) as u32).map(|x| x.id);
                    // ponytail: related fields (other base tables) are not editable yet.
                    if ft != Some(t) {
                        painter.rect_stroke(r, 2.0, frame, StrokeKind::Inside);
                        continue;
                    }
                    let field = field as u32;
                    let is_calc = doc
                        .db
                        .table(t)
                        .and_then(|x| x.field(field))
                        .is_some_and(|f| f.calc.is_some());
                    match (mode, record) {
                        (Mode::Find, _) => {
                            let s = self.request.entry(field).or_default();
                            ui.put(r, egui::TextEdit::singleline(s).font(font.clone()));
                        }
                        (Mode::Browse, None) => {
                            painter.rect_stroke(r, 2.0, frame, StrokeKind::Inside);
                        }
                        (Mode::Browse, Some(rec)) if is_calc => {
                            let v = doc.db.value(t, rec, field).to_text().replace('\r', " ");
                            painter.rect_stroke(r, 2.0, frame, StrokeKind::Inside);
                            painter.text(
                                r.left_center() + Vec2::new(4.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                v,
                                font.clone(),
                                Color32::BLACK,
                            );
                        }
                        (Mode::Browse, Some(rec)) => {
                            if let Some(tbl) = doc.db.tables.iter_mut().find(|x| x.id == t) {
                                let s = tbl.records[rec].values.entry(field).or_default();
                                if ui
                                    .put(r, egui::TextEdit::singleline(s).font(font.clone()))
                                    .changed()
                                {
                                    self.dirty = true;
                                }
                            }
                        }
                    }
                }
                2 => {
                    if let Some(txt) = &o.text {
                        painter.text(
                            r.left_center(),
                            egui::Align2::LEFT_CENTER,
                            txt,
                            font.clone(),
                            Color32::from_gray(90),
                        );
                    }
                }
                10 | 18 => {
                    painter.rect_stroke(
                        r,
                        4.0,
                        Stroke::new(1.0, Color32::from_gray(160)),
                        StrokeKind::Inside,
                    );
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        if self.doc.is_some() {
            egui::Panel::top("status").show(ui, |ui| {
                self.status_toolbar(ui);
                self.layout_bar(ui);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::RED, e);
            }
            if self.doc.is_none() {
                ui.centered_and_justified(|ui| {
                    if ui.button(tr("Open…")).clicked() {
                        self.open_dialog();
                    }
                });
                return;
            }
            egui::ScrollArea::both().show(ui, |ui| self.content(ui));
        });
        let title = self.doc.as_ref().map_or("OpenMaker".into(), |d| {
            d.path
                .file_stem()
                .map_or("OpenMaker".into(), |s| s.to_string_lossy().into_owned())
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

fn main() -> eframe::Result {
    let mut app = App::default();
    if let Some(p) = std::env::args_os().nth(1) {
        app.open(p.into());
    }
    eframe::run_native(
        "OpenMaker",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1060.0, 980.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            // FileMaker's desktop UI is light.
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(app))
        }),
    )
}
