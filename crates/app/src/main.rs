//! OpenMaker desktop application (egui).
//!
//! Current scope: open a `.fmp12` file read-only, choose a layout, browse
//! records in form view with the layout drawn at its real coordinates.

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use fmp12::{graph, layouts, schema};

mod i18n;
use i18n::tr;

struct Doc {
    path: PathBuf,
    tables: Vec<schema::Table>,
    occurrences: Vec<graph::TableOccurrence>,
    layouts: Vec<layouts::Layout>,
    records: BTreeMap<u32, schema::Records>,
}

impl Doc {
    fn open(path: PathBuf) -> Result<Self, fmp12::Error> {
        let f = fmp12::File::open(&path)?;
        let tables = schema::tables(&f)?;
        let mut records = BTreeMap::new();
        for t in &tables {
            records.insert(t.id, schema::records(&f, t)?);
        }
        Ok(Doc {
            path,
            occurrences: graph::table_occurrences(&f)?,
            layouts: layouts::layouts(&f)?
                .into_iter()
                .filter(|l| l.table_occurrence != 0)
                .collect(),
            tables,
            records,
        })
    }

    /// Base table behind a layout's table occurrence (XML-style id).
    fn base_table(&self, to_xml_id: u64) -> Option<&schema::Table> {
        let to = self
            .occurrences
            .iter()
            .find(|t| t.id as u64 == to_xml_id & 0xFFF)?;
        self.tables.iter().find(|t| t.id == to.base_table)
    }
}

#[derive(Default)]
struct App {
    doc: Option<Doc>,
    error: Option<String>,
    layout: usize,
    record: usize,
}

impl App {
    fn open_dialog(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("FileMaker", &["fmp12"])
            .pick_file()
        else {
            return;
        };
        self.open(p);
    }

    fn open(&mut self, p: PathBuf) {
        match Doc::open(p) {
            Ok(d) => {
                self.doc = Some(d);
                self.layout = 0;
                self.record = 0;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
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
                    self.doc = None;
                }
                ui.separator();
                if ui.button(tr("Exit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            for m in [
                "Edit", "View", "Insert", "Format", "Records", "Scripts", "Tools", "Window", "Help",
            ] {
                ui.menu_button(tr(m), |ui| {
                    ui.label(tr("Not implemented yet"));
                });
            }
        });
    }

    /// FileMaker's status toolbar: record navigation and found-set counts.
    fn status_toolbar(&mut self, ui: &mut egui::Ui, total: usize) {
        ui.horizontal(|ui| {
            if ui.button("<").clicked() && self.record > 0 {
                self.record -= 1;
            }
            if ui.button(">").clicked() && self.record + 1 < total {
                self.record += 1;
            }
            let shown = if total == 0 { 0 } else { self.record + 1 };
            ui.label(format!("{shown}"));
            ui.separator();
            ui.vertical(|ui| {
                ui.label(format!("{total}"));
                ui.label(format!("{} ({})", tr("Total"), tr("Unsorted")));
            });
            ui.separator();
            for b in [
                "Show All",
                "New Record",
                "Delete Record",
                "Find",
                "Sort",
                "Share",
            ] {
                ui.add_enabled(false, egui::Button::new(tr(b)));
            }
        });
    }

    fn layout_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        ui.horizontal(|ui| {
            ui.label(tr("Layout:"));
            let current = doc.layouts.get(self.layout).map_or("", |l| l.name.as_str());
            egui::ComboBox::from_id_salt("layout")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, l) in doc.layouts.iter().enumerate() {
                        if ui.selectable_label(i == self.layout, &l.name).clicked() {
                            self.layout = i;
                            self.record = 0;
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
    }
}

/// Draws one layout in form view at its stored coordinates.
fn paint_layout(ui: &mut egui::Ui, doc: &Doc, layout: &layouts::Layout, record: usize) {
    let table = doc.base_table(layout.table_occurrence);
    let rec = table
        .and_then(|t| doc.records.get(&t.id))
        .and_then(|r| r.values().nth(record));
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
    let at = |b: &layouts::Bounds| {
        Rect::from_min_max(
            origin + Vec2::new(b.left as f32, b.top as f32),
            origin + Vec2::new(b.right as f32, b.bottom as f32),
        )
    };
    painter.rect_filled(resp.rect, 0.0, Color32::from_gray(245));
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
    for o in &layout.objects {
        let r = at(&o.bounds);
        match o.kind {
            // Field (edit box, pop-up, …)
            1 => {
                let (to, field, _) = o.field.unwrap_or_default();
                let value = rec
                    .filter(|_| doc.base_table(to).map(|t| t.id) == table.map(|t| t.id))
                    .and_then(|r| r.get(&(field as u32)))
                    .map_or("", |s| s.as_str());
                painter.rect_stroke(
                    r,
                    2.0,
                    Stroke::new(1.0, Color32::from_gray(190)),
                    StrokeKind::Inside,
                );
                painter.text(
                    r.left_center() + Vec2::new(4.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    value.replace('\r', " "),
                    font.clone(),
                    Color32::BLACK,
                );
            }
            // Static text
            2 => {
                if let Some(t) = &o.text {
                    painter.text(
                        r.left_center(),
                        egui::Align2::LEFT_CENTER,
                        t,
                        font.clone(),
                        Color32::from_gray(90),
                    );
                }
            }
            // Button / button bar
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let total = self
            .doc
            .as_ref()
            .and_then(|d| {
                let l = d.layouts.get(self.layout)?;
                let t = d.base_table(l.table_occurrence)?;
                Some(d.records.get(&t.id)?.len())
            })
            .unwrap_or(0);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        if self.doc.is_some() {
            egui::Panel::top("status").show(ui, |ui| {
                self.status_toolbar(ui, total);
                self.layout_bar(ui);
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::RED, e);
            }
            match &self.doc {
                None => {
                    ui.centered_and_justified(|ui| {
                        if ui.button(tr("Open…")).clicked() {
                            self.open_dialog();
                        }
                    });
                }
                Some(doc) => {
                    if let Some(l) = doc.layouts.get(self.layout) {
                        egui::ScrollArea::both()
                            .show(ui, |ui| paint_layout(ui, doc, l, self.record));
                    }
                }
            }
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
        Box::new(|_| Ok(Box::new(app))),
    )
}
