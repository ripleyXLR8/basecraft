//! Table view as in FileMaker: one column per field of the layout's body,
//! one row per record; columns resized by dragging their right edge,
//! sorted by a click on their header (again: descending), fields added or
//! hidden from the header's menu, an optional total row; the current
//! record highlighted. Rows off screen are not drawn.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, Stroke};
use engine::Order;
use fmp12::layouts::Layout;

use crate::{App, commit_field, i18n, tr};

pub fn tv(key: &'static str) -> &'static str {
    if !i18n::french() {
        return tr(key);
    }
    match key {
        "Sort Ascending" => "Tri croissant",
        "Sort Descending" => "Tri décroissant",
        "Unsort" => "Annuler le tri",
        "Hide Field" => "Supprimer la rubrique",
        "Add Field" => "Ajouter une rubrique",
        "Show Total" => "Afficher le total",
        "Total" => "Total",
        k => tr(k),
    }
}

/// Table view settings, per layout id.
#[derive(Default)]
pub struct State {
    widths: BTreeMap<(u32, u32), f32>,
    hidden: BTreeSet<(u32, u32)>,
    pub(crate) added: BTreeMap<u32, Vec<u32>>,
    totals: BTreeSet<(u32, u32)>,
    /// Column the found set is sorted by from its header, and the order.
    sorted_by: Option<(u32, Order)>,
}

const ROW: f32 = 22.0;

/// What a click in the header does, once the table is drawn.
type Action = Box<dyn FnOnce(&mut App)>;

impl App {
    /// The table view's columns: the layout's fields of its table, left
    /// to right, then the fields added, without the hidden ones.
    pub fn table_columns(&self, t: u32, layout: &Layout) -> Vec<u32> {
        let Some(d) = self.doc.as_ref() else {
            return Vec::new();
        };
        let mut cols: Vec<(f64, u32)> = layout
            .objects
            .iter()
            .filter(|o| o.kind == 1)
            .filter_map(|o| {
                let (to, f, _) = o.field?;
                (d.db.table_of((to & 0xFFF) as u32)?.id == t).then_some((o.bounds.left, f as u32))
            })
            .collect();
        cols.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut v: Vec<u32> = cols.into_iter().map(|c| c.1).collect();
        v.extend(self.views.added.get(&layout.id).into_iter().flatten());
        let mut seen = BTreeSet::new();
        v.retain(|f| seen.insert(*f) && !self.views.hidden.contains(&(layout.id, *f)));
        v
    }

    /// Records › Sort by a column header (ascending, then descending).
    pub fn sort_by(&mut self, f: u32, order: Order) {
        if !self.commit() {
            return;
        }
        let Some(t) = self.table() else { return };
        let mut found = self.found(t).clone();
        if let Some(d) = &self.doc {
            engine::sort(&d.db, t, &mut found, &[(f, order)]);
        }
        self.last_sort = vec![(f, order)];
        self.found.insert(t, found);
        self.sorted.insert(t, true);
        self.current.insert(t, 0);
        self.views.sorted_by = Some((f, order));
    }

    pub fn table_view(&mut self, ui: &mut egui::Ui, t: u32, layout: &Layout, found: &[usize]) {
        let lid = layout.id;
        let cols = self.table_columns(t, layout);
        let cur = self.current.get(&t).copied().unwrap_or(0);
        let sorted = self.sorted.get(&t).copied().unwrap_or(false);
        let Some(d) = self.doc.as_ref() else { return };
        let Some(tbl) = d.db.table(t) else { return };
        let names: BTreeMap<u32, (String, bool, bool)> = tbl
            .fields
            .iter()
            .map(|f| {
                let num = f.data_type == fmp12::schema::DataType::Number;
                (f.id, (f.name.clone(), f.calc.is_some(), num))
            })
            .collect();
        let width = |s: &State, f: u32| s.widths.get(&(lid, f)).copied().unwrap_or(140.0);
        let mut action: Option<Action> = None;
        // Header: names, sort arrows, resize handles, the column menu.
        let (hdr, _) = ui.allocate_exact_size(
            egui::vec2(
                cols.iter().map(|f| width(&self.views, *f)).sum::<f32>() + 30.0,
                ROW,
            ),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(hdr.expand(2.0));
        painter.rect_filled(hdr, 0.0, Color32::from_gray(236));
        let mut x = hdr.left() + 30.0;
        for &f in &cols {
            let w = width(&self.views, f);
            let cell = egui::Rect::from_min_size(egui::pos2(x, hdr.top()), egui::vec2(w, ROW));
            let (name, _, _) = names.get(&f).cloned().unwrap_or_default();
            let arrow = match self.views.sorted_by {
                Some((sf, o)) if sf == f && sorted => {
                    if o == Order::Descending {
                        " ▼"
                    } else {
                        " ▲"
                    }
                }
                _ => "",
            };
            painter.text(
                cell.left_center() + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{name}{arrow}"),
                egui::FontId::proportional(13.0),
                Color32::from_gray(40),
            );
            painter.line_segment(
                [cell.right_top(), cell.right_bottom()],
                Stroke::new(1.0, Color32::from_gray(200)),
            );
            let r = ui
                .interact(cell, ui.scope_id().with(("col", f)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let label = name.clone();
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColumnHeader, true, &label));
            if r.clicked() {
                let o = match self.views.sorted_by {
                    Some((sf, Order::Ascending)) if sf == f && sorted => Order::Descending,
                    _ => Order::Ascending,
                };
                action = Some(Box::new(move |a: &mut App| a.sort_by(f, o)));
            }
            r.context_menu(|ui| {
                if ui.button(tv("Sort Ascending")).clicked() {
                    action = Some(Box::new(move |a: &mut App| a.sort_by(f, Order::Ascending)));
                    ui.close();
                }
                if ui.button(tv("Sort Descending")).clicked() {
                    action = Some(Box::new(move |a: &mut App| a.sort_by(f, Order::Descending)));
                    ui.close();
                }
                if ui.button(tv("Unsort")).clicked() {
                    action = Some(Box::new(|a: &mut App| {
                        a.unsort();
                        a.views.sorted_by = None;
                    }));
                    ui.close();
                }
                ui.separator();
                let mut total = self.views.totals.contains(&(lid, f));
                if ui.checkbox(&mut total, tv("Show Total")).clicked() {
                    action = Some(Box::new(move |a: &mut App| {
                        if !a.views.totals.remove(&(lid, f)) {
                            a.views.totals.insert((lid, f));
                        }
                    }));
                    ui.close();
                }
                if ui.button(tv("Hide Field")).clicked() {
                    action = Some(Box::new(move |a: &mut App| {
                        a.views.hidden.insert((lid, f));
                    }));
                    ui.close();
                }
                ui.menu_button(tv("Add Field"), |ui| {
                    for (id, (n, _, _)) in names.iter().filter(|(id, _)| !cols.contains(id)) {
                        if ui.button(n).clicked() {
                            let id = *id;
                            action = Some(Box::new(move |a: &mut App| {
                                a.views.hidden.remove(&(lid, id));
                                a.views.added.entry(lid).or_default().push(id);
                            }));
                            ui.close();
                        }
                    }
                });
            });
            // Resize: drag the right edge.
            let edge = egui::Rect::from_center_size(cell.right_center(), egui::vec2(6.0, ROW));
            let drag = ui
                .interact(edge, ui.scope_id().with(("resize", f)), egui::Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if drag.dragged() {
                let nw = (w + drag.drag_delta().x).max(30.0);
                self.views.widths.insert((lid, f), nw);
            }
            x += w;
        }
        // Rows.
        let sort_keys: Vec<u32> = if sorted {
            self.last_sort.iter().map(|(f, _)| *f).collect()
        } else {
            Vec::new()
        };
        let widths: Vec<f32> = cols.iter().map(|f| width(&self.views, *f)).collect();
        let Some(d) = self.doc.as_mut() else { return };
        let mut clicked_row = None;
        let mut dirty = false;
        for (i, &rec) in found.iter().enumerate() {
            let (row, _) = ui.allocate_exact_size(
                egui::vec2(widths.iter().sum::<f32>() + 30.0, ROW),
                egui::Sense::hover(),
            );
            if !ui.is_rect_visible(row) {
                continue;
            }
            let p = ui.painter();
            if i == cur {
                p.rect_filled(row, 0.0, Color32::from_rgb(220, 232, 250));
                p.rect_filled(
                    egui::Rect::from_min_size(row.min, egui::vec2(4.0, ROW)),
                    0.0,
                    Color32::from_rgb(40, 40, 40),
                );
            } else if i % 2 == 1 {
                p.rect_filled(row, 0.0, Color32::from_gray(248));
            }
            p.text(
                row.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(11.0),
                Color32::from_gray(140),
            );
            if ui.input(|x| x.pointer.any_pressed()) && ui.rect_contains_pointer(row) {
                clicked_row = Some(i);
            }
            let mut x = row.left() + 30.0;
            for (k, &f) in cols.iter().enumerate() {
                let w = widths[k];
                let cell = egui::Rect::from_min_size(egui::pos2(x, row.top()), egui::vec2(w, ROW))
                    .shrink2(egui::vec2(2.0, 1.0));
                x += w;
                let (_, is_calc, _) = names.get(&f).cloned().unwrap_or_default();
                if is_calc || !d.db.can_edit_field(t, rec, f) {
                    // As the window sees it: summaries over the found set.
                    let fs = engine::FoundSet {
                        table: t,
                        records: found,
                        sort: &sort_keys,
                    };
                    let v = d.db.value_in(t, rec, f, 1, Some(fs)).to_text();
                    ui.put(cell, egui::Label::new(v).truncate());
                    continue;
                }
                let Some(s) =
                    d.db.tables
                        .iter_mut()
                        .find(|x| x.id == t)
                        .and_then(|x| x.raw_mut(rec, f))
                else {
                    continue;
                };
                let r = ui.put(
                    cell,
                    egui::TextEdit::singleline(s)
                        .id(ui.scope_id().with(("cell", rec, f)))
                        .frame(egui::Frame::NONE),
                );
                if r.gained_focus() {
                    clicked_row = Some(i);
                }
                if r.changed() {
                    dirty = true;
                    d.db.touch(t, rec);
                }
                commit_field(ui, &mut d.db, (t, rec, f), r.changed(), r.lost_focus());
            }
        }
        // Total row.
        let totals: Vec<(usize, u32)> = cols
            .iter()
            .enumerate()
            .filter(|(_, f)| self.views.totals.contains(&(lid, **f)))
            .map(|(k, f)| (k, *f))
            .collect();
        if !totals.is_empty() {
            let (row, _) = ui.allocate_exact_size(
                egui::vec2(widths.iter().sum::<f32>() + 30.0, ROW),
                egui::Sense::hover(),
            );
            ui.painter().rect_filled(row, 0.0, Color32::from_gray(236));
            for (k, f) in totals {
                let x = row.left() + 30.0 + widths[..k].iter().sum::<f32>();
                let num = names.get(&f).is_some_and(|n| n.2);
                let text = if num {
                    let sum: f64 = found
                        .iter()
                        .filter_map(|r| d.db.value(t, *r, f).to_text().trim().parse::<f64>().ok())
                        .sum();
                    format!("{} {sum}", tv("Total"))
                } else {
                    let n = found
                        .iter()
                        .filter(|r| !d.db.value(t, **r, f).to_text().is_empty())
                        .count();
                    format!("{} {n}", tv("Count"))
                };
                let cell = egui::Rect::from_min_size(
                    egui::pos2(x + 4.0, row.top()),
                    egui::vec2(widths[k] - 4.0, ROW),
                );
                ui.put(cell, egui::Label::new(egui::RichText::new(text).strong()));
            }
        }
        if dirty {
            self.dirty = true;
        }
        if let Some(i) = clicked_row.filter(|i| *i != cur) {
            self.go_to(i);
        }
        if let Some(a) = action {
            a(self);
        }
    }
}
