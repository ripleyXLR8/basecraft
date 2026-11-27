//! OpenMaker desktop application (egui).
//!
//! Browse and Find modes over a `.fmp12` file: layouts drawn at their real
//! coordinates, editable fields, new/delete record, find requests, sort.
//! Changes live in memory (saving to `.fmp12` is milestone M7).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use engine::{Database, Order, Request};
use fmp12::layouts::Layout;

mod i18n;
mod look;
use i18n::tr;
use look::Look;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Find,
}

struct Doc {
    /// File name shown in the title bar.
    name: String,
    db: Database,
    /// Layouts that have a table occurrence (folders excluded).
    layouts: Vec<Layout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Form,
    List,
    Table,
}

/// A file picked asynchronously (browser), waiting to be opened.
type Incoming = Rc<RefCell<Option<(String, Vec<u8>)>>>;

#[derive(Default)]
struct App {
    incoming: Incoming,
    view: View,
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
    /// Front panel chosen by the user, per tab control id.
    tabs: BTreeMap<u64, u64>,
}

impl App {
    fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Browse)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self, _ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("FileMaker", &["fmp12"])
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    /// In the browser the picker is asynchronous: the file lands in
    /// `incoming` and is opened on the next frame.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let slot = self.incoming.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new()
                .add_filter("FileMaker", &["fmp12"])
                .pick_file()
                .await
            {
                let data = h.read().await;
                *slot.borrow_mut() = Some((h.file_name(), data));
                ctx.request_repaint();
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_path(&mut self, p: &std::path::Path) {
        match std::fs::read(p) {
            Ok(b) => {
                let name = p
                    .file_stem()
                    .map_or("OpenMaker".into(), |s| s.to_string_lossy().into_owned());
                self.open(name, b);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn open(&mut self, name: String, bytes: Vec<u8>) {
        let name = name.trim_end_matches(".fmp12").to_string();
        match fmp12::File::from_bytes(bytes).and_then(|f| Database::load(&f)) {
            Ok(db) => {
                let layouts = db
                    .layouts
                    .iter()
                    .filter(|l| l.table_occurrence != 0)
                    .cloned()
                    .collect();
                *self = App {
                    doc: Some(Doc { name, db, layouts }),
                    incoming: self.incoming.clone(),
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
                    self.open_dialog(&ui.ctx().clone());
                }
                if ui.button(tr("Close")).clicked() {
                    ui.close();
                    *self = App {
                        incoming: self.incoming.clone(),
                        ..Default::default()
                    };
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui.button(tr("Exit")).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
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
        if self.doc.is_none() {
            return;
        }
        let names: Vec<String> = self
            .doc
            .as_ref()
            .unwrap()
            .layouts
            .iter()
            .map(|l| l.name.clone())
            .collect();
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(tr("Layout:"));
            let current = names.get(self.layout).map_or("", |n| n.as_str());
            egui::ComboBox::from_id_salt("layout")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if ui.selectable_label(i == self.layout, n).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            ui.label(tr("View As:"));
            for (v, label) in [
                (View::Form, "Form"),
                (View::List, "List"),
                (View::Table, "Table"),
            ] {
                if ui.selectable_label(self.view == v, tr(label)).clicked() {
                    self.view = v;
                }
            }
            ui.add_enabled(false, egui::Button::new(tr("Preview")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled(false, egui::Button::new(tr("Edit Layout")));
            });
        });
        if let Some(i) = chosen {
            self.layout = i;
        }
    }

    /// Draws the current layout: form (current record), list (body part
    /// repeated per record) or table (one row per record).
    fn content(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.table() else { return };
        let record = self.current_record();
        let found = self.found(t).clone();
        let mode = self.mode();
        let view = self.view;
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(layout) = doc.layouts.get(self.layout).cloned() else {
            return;
        };
        let theme = doc
            .db
            .themes
            .iter()
            .find(|x| x.number == layout.theme)
            .cloned();
        let looks: Vec<Look> = layout
            .objects
            .iter()
            .map(|o| {
                look::resolve(
                    theme.as_ref(),
                    &fmp12::theme::ObjectStyle::parse(&o.style_raw),
                )
            })
            .collect();
        let layout_look = look::resolve(
            theme.as_ref(),
            &fmp12::theme::ObjectStyle::of_layout(&layout),
        );
        if view == View::Table && mode == Mode::Browse {
            table_view(ui, doc, t, &layout, &found, &mut self.dirty);
            return;
        }
        let (btop, bh) = layout
            .parts
            .iter()
            .find(|p| p.kind == 4)
            .map_or((0.0, 0.0), |p| (p.top, p.height));
        let rows: Vec<Option<usize>> = match (view, mode) {
            (View::List, Mode::Browse) => found.iter().map(|r| Some(*r)).collect(),
            _ => vec![record],
        };
        let extra = rows.len().saturating_sub(1) as f64 * bh;
        let height = layout
            .parts
            .iter()
            .map(|p| p.top + p.height)
            .fold(0.0, f64::max)
            + extra;
        let (resp, painter) = ui.allocate_painter(
            Vec2::new(layout.width as f32, height as f32),
            egui::Sense::hover(),
        );
        let origin = resp.rect.min;
        let bg = if mode == Mode::Find {
            Color32::from_rgb(235, 240, 250)
        } else {
            layout_look.background.unwrap_or(Color32::WHITE)
        };
        painter.rect_filled(resp.rect, 0.0, bg);
        // Parts below the body move down by the extra list rows.
        let shift = |y: f64| if y >= btop + bh { y + extra } else { y };
        for p in &layout.parts {
            if mode == Mode::Browse {
                let pl = look::resolve(
                    theme.as_ref(),
                    &fmp12::theme::ObjectStyle::parse(&p.style_raw),
                );
                if let Some(c) = pl.background {
                    let (top, bottom) = if p.kind == 4 {
                        (p.top, p.top + p.height + extra)
                    } else {
                        (shift(p.top), shift(p.top + p.height))
                    };
                    painter.rect_filled(
                        Rect::from_min_max(
                            origin + Vec2::new(0.0, top as f32),
                            origin + Vec2::new(layout.width as f32, bottom as f32),
                        ),
                        0.0,
                        c,
                    );
                }
            }
            let y = origin.y + shift(p.top + p.height) as f32;
            painter.line_segment(
                [
                    Pos2::new(origin.x, y),
                    Pos2::new(origin.x + layout.width as f32, y),
                ],
                Stroke::new(0.5, Color32::from_gray(210)),
            );
        }
        let ctx = DrawCtx { table: t, mode };
        let layout_to = (layout.table_occurrence & 0xFFF) as u32;
        let by_id: BTreeMap<u64, usize> = layout
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id, i))
            .collect();
        let ancestors = |o: &fmp12::layouts::LayoutObject| {
            let mut v = Vec::new();
            let mut p = o.parent;
            while let Some(id) = p {
                let Some(&i) = by_id.get(&id) else { break };
                v.push(&layout.objects[i]);
                p = layout.objects[i].parent;
            }
            v
        };
        // Front panel of each tab control: the user's choice, else field 6, else the first panel.
        let front = |tc: &fmp12::layouts::LayoutObject, tabs: &BTreeMap<u64, u64>| {
            tabs.get(&tc.id).copied().or(tc.active_child).or_else(|| {
                layout
                    .objects
                    .iter()
                    .find(|x| x.parent == Some(tc.id) && x.kind == 12)
                    .map(|x| x.id)
            })
        };
        let mut clicked_tab = None;
        for (o, lk) in layout.objects.iter().zip(&looks) {
            let anc = ancestors(o);
            // Hidden behind another tab, or drawn by its portal.
            let hidden = anc.iter().any(|a| {
                a.kind == 12
                    && a.parent
                        .and_then(|tc| by_id.get(&tc))
                        .is_some_and(|&i| front(&layout.objects[i], &self.tabs) != Some(a.id))
            });
            if hidden || anc.iter().any(|a| a.kind == 9) {
                continue;
            }
            let ab = layout.absolute(o);
            let in_body = ab.top >= btop && ab.top < btop + bh;
            let instances: Vec<(f64, Option<usize>)> = if in_body {
                rows.iter()
                    .enumerate()
                    .map(|(i, r)| (i as f64 * bh, *r))
                    .collect()
            } else {
                vec![(shift(ab.top) - ab.top, record)]
            };
            for (dy, rec) in instances {
                let r = Rect::from_min_max(
                    origin + Vec2::new(ab.left as f32, (ab.top + dy) as f32),
                    origin + Vec2::new(ab.right as f32, (ab.bottom + dy) as f32),
                );
                match o.kind {
                    // Tab control: one clickable tab per panel.
                    11 => {
                        lk.paint_box(&painter, r);
                        let active = front(o, &self.tabs);
                        let mut x = r.left();
                        for p in layout.objects.iter().filter(|p| p.parent == Some(o.id)) {
                            let label = p.text.clone().unwrap_or_default();
                            let w = (label.chars().count() as f32 * 8.0 + 32.0).max(60.0);
                            let tab =
                                Rect::from_min_size(egui::pos2(x, r.top()), Vec2::new(w, 30.0));
                            let on = active == Some(p.id);
                            painter.rect_filled(
                                tab,
                                0.0,
                                if on {
                                    Color32::WHITE
                                } else {
                                    Color32::from_gray(232)
                                },
                            );
                            painter.rect_stroke(
                                tab,
                                0.0,
                                Stroke::new(1.0, Color32::from_gray(200)),
                                StrokeKind::Inside,
                            );
                            painter.text(
                                tab.center(),
                                egui::Align2::CENTER_CENTER,
                                &label,
                                lk.font(),
                                Color32::from_gray(if on { 20 } else { 110 }),
                            );
                            let id = ui.scope_id().with(("tab", o.id, p.id));
                            if ui.interact(tab, id, egui::Sense::click()).clicked() {
                                clicked_tab = Some((o.id, p.id));
                            }
                            x += w;
                        }
                    }
                    // Portal: one row per related record.
                    9 => {
                        lk.paint_box(&painter, r);
                        let Some((pto, nrows, first)) = o.portal else {
                            continue;
                        };
                        let pto = (pto & 0xFFF) as u32;
                        let Some(ptable) = doc.db.table_of(pto).map(|x| x.id) else {
                            continue;
                        };
                        let related = match (mode, rec) {
                            (Mode::Browse, Some(rr)) => doc.db.related(layout_to, rr, pto),
                            _ => Vec::new(),
                        };
                        let nrows = nrows.max(1) as usize;
                        let row_h = (o.bounds.bottom - o.bounds.top) / nrows as f64;
                        let children: Vec<usize> = layout
                            .objects
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.parent == Some(o.id))
                            .map(|(i, _)| i)
                            .collect();
                        for row in 0..nrows {
                            let related_rec =
                                related.get(first.saturating_sub(1) as usize + row).copied();
                            let y0 = r.top() + (row as f64 * row_h) as f32;
                            if row > 0 {
                                painter.line_segment(
                                    [egui::pos2(r.left(), y0), egui::pos2(r.right(), y0)],
                                    Stroke::new(0.5, Color32::from_gray(220)),
                                );
                            }
                            let Some(rr) = related_rec else { continue };
                            for &ci in &children {
                                let c = &layout.objects[ci];
                                let cr = Rect::from_min_size(
                                    egui::pos2(
                                        r.left() + c.bounds.left as f32,
                                        y0 + c.bounds.top as f32,
                                    ),
                                    Vec2::new(
                                        (c.bounds.right - c.bounds.left) as f32,
                                        (c.bounds.bottom - c.bounds.top) as f32,
                                    ),
                                );
                                draw_object(
                                    ui,
                                    &painter,
                                    doc,
                                    &ctx,
                                    c,
                                    &looks[ci],
                                    cr,
                                    Some((ptable, Some(rr))),
                                    &mut self.request,
                                    &mut self.dirty,
                                );
                            }
                        }
                    }
                    _ => {
                        if let Some(l) = &o.label {
                            let text = doc.db.eval_tokens(l, t, layout_to, rec).to_text();
                            lk.paint_box(&painter, r);
                            lk.paint_text(&painter, r, &text);
                            continue;
                        }
                        let owner = owner_of(doc, o, t, layout_to, rec);
                        draw_object(
                            ui,
                            &painter,
                            doc,
                            &ctx,
                            o,
                            lk,
                            r,
                            owner,
                            &mut self.request,
                            &mut self.dirty,
                        );
                    }
                }
            }
        }
        if let Some((tc, p)) = clicked_tab {
            self.tabs.insert(tc, p);
        }
    }
}

/// Table and record whose value a field object shows: the layout's own
/// record, or the first related record for a field of another occurrence.
fn owner_of(
    doc: &Doc,
    o: &fmp12::layouts::LayoutObject,
    t: u32,
    layout_to: u32,
    rec: Option<usize>,
) -> Option<(u32, Option<usize>)> {
    let (to, _, _) = o.field?;
    let to = (to & 0xFFF) as u32;
    let ft = doc.db.table_of(to)?.id;
    if to == layout_to || ft == t && doc.db.table_of(layout_to).map(|x| x.id) == Some(ft) {
        return Some((t, rec));
    }
    let first = rec.and_then(|r| doc.db.related(layout_to, r, to).first().copied());
    Some((ft, first))
}

struct DrawCtx {
    table: u32,
    mode: Mode,
}

#[allow(clippy::too_many_arguments)]
fn draw_object(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &mut Doc,
    c: &DrawCtx,
    o: &fmp12::layouts::LayoutObject,
    lk: &Look,
    r: Rect,
    owner: Option<(u32, Option<usize>)>,
    request: &mut BTreeMap<u32, String>,
    dirty: &mut bool,
) {
    lk.paint_box(painter, r);
    if o.kind != 1 {
        if let Some(txt) = &o.text {
            lk.paint_text(painter, r, txt);
        }
        return;
    }
    let (Some((_, field, _)), Some((t, record))) = (o.field, owner) else {
        return;
    };
    let field = field as u32;
    let is_calc = doc
        .db
        .table(t)
        .and_then(|x| x.field(field))
        .is_some_and(|f| f.calc.is_some());
    let inner = lk.content_rect(r);
    match (c.mode, record) {
        (Mode::Find, _) if t == c.table => {
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_gray(150)),
                StrokeKind::Inside,
            );
            let s = request.entry(field).or_default();
            ui.put(inner, lk.edit(s));
        }
        (Mode::Find, _) | (Mode::Browse, None) => {}
        (Mode::Browse, Some(rec)) if is_calc => {
            let v = doc.db.value(t, rec, field).to_text().replace('\r', " ");
            lk.paint_text(painter, r, &v);
        }
        (Mode::Browse, Some(rec)) => {
            // Pop-up menus and drop-down lists offer their value list.
            let choices = match (o.display, o.value_list) {
                (1 | 2, Some(vl)) => Some(doc.db.value_list_values(vl)),
                _ => None,
            };
            if let Some(tbl) = doc.db.tables.iter_mut().find(|x| x.id == t) {
                let s = tbl.records[rec].values.entry(field).or_default();
                match choices {
                    Some(values) => {
                        let mut picked = None;
                        ui.put(inner, |ui: &mut egui::Ui| {
                            egui::ComboBox::from_id_salt(("vl", o.id, rec))
                                .selected_text(s.as_str())
                                .width(inner.width())
                                .show_ui(ui, |ui| {
                                    for v in &values {
                                        if ui.selectable_label(s == v, v).clicked() {
                                            picked = Some(v.clone());
                                        }
                                    }
                                })
                                .response
                        });
                        if let Some(v) = picked {
                            *s = v;
                            *dirty = true;
                        }
                    }
                    None => {
                        if ui.put(inner, lk.edit(s)).changed() {
                            *dirty = true;
                        }
                    }
                }
            }
        }
    }
}

/// Table view: one column per field of the layout's body, one row per record.
fn table_view(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    t: u32,
    layout: &Layout,
    found: &[usize],
    dirty: &mut bool,
) {
    let mut cols: Vec<(f64, u32)> = layout
        .objects
        .iter()
        .filter(|o| o.kind == 1)
        .filter_map(|o| {
            let (to, f, _) = o.field?;
            (doc.db.table_of((to & 0xFFF) as u32)?.id == t).then_some((o.bounds.left, f as u32))
        })
        .collect();
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    cols.dedup_by_key(|c| c.1);
    egui::Grid::new("table_view").striped(true).show(ui, |ui| {
        for (_, f) in &cols {
            let name = doc
                .db
                .table(t)
                .and_then(|x| x.field(*f))
                .map_or(String::new(), |f| f.name.clone());
            ui.strong(name);
        }
        ui.end_row();
        for &rec in found {
            for (_, f) in &cols {
                let is_calc = doc
                    .db
                    .table(t)
                    .and_then(|x| x.field(*f))
                    .is_some_and(|f| f.calc.is_some());
                if is_calc {
                    ui.label(doc.db.value(t, rec, *f).to_text());
                } else if let Some(tbl) = doc.db.tables.iter_mut().find(|x| x.id == t) {
                    let s = tbl.records[rec].values.entry(*f).or_default();
                    if ui.text_edit_singleline(s).changed() {
                        *dirty = true;
                    }
                }
            }
            ui.end_row();
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let picked = self.incoming.borrow_mut().take();
        if let Some((name, bytes)) = picked {
            self.open(name, bytes);
        }
        // Drag and drop: read synchronously on desktop, asynchronously in the browser.
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = f
                .path()
                .file_name()
                .map_or("OpenMaker".into(), |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            match f.bytes() {
                Ok(b) => self.open(name, b),
                Err(e) => self.error = Some(e),
            }
            #[cfg(target_arch = "wasm32")]
            {
                let slot = self.incoming.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(b) = f.bytes_async().await {
                        *slot.borrow_mut() = Some((name, b));
                        ctx.request_repaint();
                    }
                });
            }
        }
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
                    ui.vertical_centered(|ui| {
                        if ui.button(tr("Open…")).clicked() {
                            self.open_dialog(&ctx);
                        }
                        ui.label(tr("or drop a .fmp12 file here"));
                    });
                });
                return;
            }
            egui::ScrollArea::both().show(ui, |ui| self.content(ui));
        });
        let title = self
            .doc
            .as_ref()
            .map_or("OpenMaker".to_string(), |d| d.name.clone());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

fn creator(app: App) -> eframe::AppCreator<'static> {
    Box::new(|cc| {
        // FileMaker's desktop UI is light.
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        Ok(Box::new(app))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let mut app = App::default();
    if let Some(p) = std::env::args_os().nth(1) {
        app.open_path(std::path::Path::new(&p));
    }
    eframe::run_native(
        "OpenMaker",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1060.0, 980.0]),
            ..Default::default()
        },
        creator(app),
    )
}

/// Browser entry point: runs in the `<canvas id="openmaker">` of index.html.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("openmaker"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("canvas #openmaker");
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                creator(App::default()),
            )
            .await
            .expect("failed to start OpenMaker");
    });
}
