//! The Launch Center (« Centre de lancement »): shown while no file is
//! open, and from File › Launch Center. Favorites, Recent files, and
//! Create (a blank file, or a file converted from CSV, tab-separated text
//! or Excel). FileMaker's starter templates are not reproduced.

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};
use engine::Database;
use engine::export::Format;

use crate::{App, i18n, prefs};

pub fn t(key: &'static str) -> &'static str {
    if !i18n::french() {
        return key;
    }
    match key {
        "Launch Center" => "Centre de lancement",
        "Favorites" => "Favoris",
        "Recent" => "Récent",
        "Create" => "Créer",
        "Name" => "Nom",
        "Location" => "Emplacement",
        "Add App" => "Ajouter l'app",
        "Remove from Favorites" => "Supprimer des favoris",
        "Blank" => "Vide",
        "Convert" => "Convertir",
        "Untitled" => "Sans titre",
        "An empty app with one table and one layout, ready for your fields." => {
            "App vide contenant une table et un modèle, prête pour vos rubriques."
        }
        "Creates an app from a CSV, tab-separated or Excel file: one field per column, one record per row." => {
            "Crée une app à partir d'un fichier CSV, texte tabulé ou Excel : une rubrique par colonne, un enregistrement par ligne."
        }
        _ => key,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Favorites,
    Recent,
    Create,
}

#[derive(Default)]
pub struct State {
    /// `None`: Recent when there are recent files, else Create.
    pub tab: Option<Tab>,
    /// Selected row of Recent.
    pub selected: Option<usize>,
    /// Create › Convert is selected (else Blank).
    pub convert: bool,
    /// File › Launch Center while a file is open (a window of its own).
    pub open: bool,
}

enum Act {
    Open(String),
    AddFavorite,
    RemoveFavorite(usize),
    Create,
}

const SIDE_W: f32 = 95.0;
const TAB_H: f32 = 85.0;
const ROW_H: f32 = 51.0;
const RIGHT_W: f32 = 325.0;
const SIDE_BG: Color32 = Color32::from_rgb(240, 240, 240);
const SIDE_SEL: Color32 = Color32::from_rgb(217, 217, 217);
const LINE: Color32 = Color32::from_rgb(204, 204, 204);
const INK: Color32 = Color32::from_rgb(20, 20, 20);
const ICON: Color32 = Color32::from_rgb(80, 80, 80);
const LINK: Color32 = Color32::from_rgb(26, 58, 160);
const ACCENT: Color32 = Color32::from_rgb(0, 120, 212);

/// File types Create › Convert (and dropping or opening them) turns into a
/// new file.
pub fn convertible(name: &str) -> bool {
    matches!(
        Format::from_name(name),
        Some(Format::Csv | Format::Tab | Format::Merge | Format::Xlsx)
    )
}

/// A new file from a table of rows: a text field per column, named from
/// the first row, and a record per other row. Returns the file's name, its
/// database and file, and the new fields.
fn converted(
    name: &str,
    bytes: &[u8],
) -> Result<(String, Database, fmp12::File, Vec<u32>), String> {
    let format = Format::from_name(name)
        .filter(|_| convertible(name))
        .ok_or_else(|| format!("{name}: not a CSV, tab-separated or Excel file"))?;
    let rows = engine::import::read_rows(format, bytes)?;
    let stem = name.rsplit_once('.').map_or(name, |s| s.0).to_string();
    let bytes = fmp12::schema_edit::new_file(&stem)
        .or_else(|_| fmp12::schema_edit::new_file("Table 1"))
        .map_err(|e| e.to_string())?;
    let mut file = fmp12::File::from_bytes(bytes).map_err(|e| e.to_string())?;
    let mut db = Database::load(&file).map_err(|e| e.to_string())?;
    let table = db.tables.first().map(|x| x.id).ok_or("no table")?;
    let (mut names, mut ids) = (Vec::new(), Vec::new());
    for (i, h) in rows.first().into_iter().flatten().enumerate() {
        let spec = |n: &str| fmp12::schema_edit::FieldSpec::new(n, fmp12::schema::DataType::Text);
        // Unusable names (empty, duplicate, reserved) become f1, f2…
        let (n, id) = match db.add_field(&mut file, table, &spec(h.trim())) {
            Ok(id) => (h.trim().to_string(), id),
            Err(_) => {
                let n = format!("f{}", i + 1);
                let id = db
                    .add_field(&mut file, table, &spec(&n))
                    .map_err(|e| e.to_string())?;
                (n, id)
            }
        };
        names.push(n);
        ids.push(id);
    }
    let opts = engine::import::Options {
        format,
        first_row_names: true,
        mapping: engine::import::Mapping::ByOrder(names),
        mode: engine::import::Mode::Add,
        auto_enter: true,
        ..Default::default()
    };
    engine::import::import(&mut db, table, &[], rows, &opts)?;
    Ok((stem, db, file, ids))
}

impl App {
    /// Opens a CSV, tab-separated or Excel file as a new file, in Table view.
    pub fn convert(&mut self, name: String, bytes: Vec<u8>) {
        match converted(&name, &bytes) {
            Ok((stem, db, file, fields)) => {
                self.opened(stem, db, file);
                if let Some(l) = self.doc.as_ref().and_then(|d| d.layouts.first()) {
                    self.views.added.insert(l.id, fields);
                }
                self.view = script::View::Table;
                self.dirty = true;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Create › Blank: a new file with one table, in this window or a new one.
    pub fn new_blank(&mut self) {
        match fmp12::schema_edit::new_file(t("Untitled")) {
            Ok(b) => self.open_more(t("Untitled").into(), b),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// File › Launch Center while a file is open.
    pub fn launch_window(&mut self, ctx: &egui::Context) {
        if !self.launch.open {
            return;
        }
        let mut open = true;
        crate::tool_window(
            ctx,
            "launch_center",
            t("Launch Center"),
            [960.0, 600.0],
            None,
            &mut open,
            |ui| self.launch_center(ui),
        );
        self.launch.open &= open;
    }

    /// The Launch Center, filling `ui`.
    pub fn launch_center(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, Sense::hover());
        let p = prefs::get();
        let tab = self.launch.tab.unwrap_or(if p.recent.is_empty() {
            Tab::Create
        } else {
            Tab::Recent
        });
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, Color32::WHITE);
        let side = Rect::from_min_size(full.min, vec2(SIDE_W, full.height()));
        painter.rect_filled(side, 0.0, SIDE_BG);
        painter.vline(side.right(), side.y_range(), Stroke::new(1.0, LINE));
        let tabs = [
            (Tab::Favorites, "Favorites"),
            (Tab::Recent, "Recent"),
            (Tab::Create, "Create"),
        ];
        for (i, (tb, label)) in tabs.into_iter().enumerate() {
            let r =
                Rect::from_min_size(side.min + vec2(0.0, i as f32 * TAB_H), vec2(SIDE_W, TAB_H));
            let resp = ui.interact(r, egui::Id::NULL.with(("launch_tab", i)), Sense::click());
            let sel = tab == tb;
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, sel, t(label)));
            if resp.clicked() {
                self.launch.tab = Some(tb);
            }
            let fill = if sel {
                SIDE_SEL
            } else if resp.hovered() {
                Color32::from_rgb(229, 229, 229)
            } else {
                SIDE_BG
            };
            painter.rect_filled(r.shrink2(vec2(0.0, 0.5)), 0.0, fill);
            painter.hline(side.x_range(), r.bottom(), Stroke::new(1.0, LINE));
            let c = pos2(r.center().x, r.top() + 31.0);
            match tb {
                Tab::Favorites => star_icon(&painter, c),
                Tab::Recent => clock_icon(&painter, c),
                Tab::Create => create_icon(&painter, c),
            }
            painter.text(
                pos2(r.center().x, r.top() + 65.0),
                Align2::CENTER_CENTER,
                t(label),
                FontId::proportional(12.5),
                INK,
            );
        }
        let body = Rect::from_min_max(pos2(side.right() + 1.0, full.top()), full.max);
        let mut act = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| match tab {
            Tab::Recent => self.recent_tab(ui, &p.recent, &mut act),
            Tab::Favorites => favorites_tab(ui, &p.favorites, &mut act),
            Tab::Create => self.create_tab(ui, &mut act),
        });
        match act {
            Some(Act::Open(path)) => {
                self.launch.open = false;
                #[cfg(not(target_arch = "wasm32"))]
                self.open_path(std::path::Path::new(&path));
                #[cfg(target_arch = "wasm32")]
                let _ = path;
            }
            Some(Act::AddFavorite) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(f) = rfd::FileDialog::new()
                    .add_filter("FileMaker", &["fmp12"])
                    .pick_file()
                {
                    let mut p = prefs::get();
                    let s = f.to_string_lossy().into_owned();
                    if !p.favorites.contains(&s) {
                        p.favorites.push(s);
                        prefs::set(p);
                    }
                }
            }
            Some(Act::RemoveFavorite(i)) => {
                let mut p = prefs::get();
                if i < p.favorites.len() {
                    p.favorites.remove(i);
                    prefs::set(p);
                }
            }
            Some(Act::Create) => {
                self.launch.open = false;
                if self.launch.convert {
                    self.pick_convert(ui.ctx());
                } else {
                    self.new_blank();
                }
            }
            None => {}
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn pick_convert(&mut self, _ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("BaseCraft", &["csv", "tab", "txt", "tsv", "mer", "xlsx"])
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    /// In the browser the file lands in `incoming`; `open_more` converts it.
    #[cfg(target_arch = "wasm32")]
    fn pick_convert(&mut self, ctx: &egui::Context) {
        let slot = self.incoming.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new()
                .add_filter("BaseCraft", &["csv", "tab", "txt", "tsv", "mer", "xlsx"])
                .pick_file()
                .await
            {
                let data = h.read().await;
                *slot.borrow_mut() = Some((h.file_name(), data));
                ctx.request_repaint();
            }
        });
    }

    /// Recent: one row per recent file, name and location; a double click opens.
    fn recent_tab(&mut self, ui: &mut egui::Ui, recent: &[String], act: &mut Option<Act>) {
        let r = ui.max_rect();
        let name_w = 300.0;
        let head = Rect::from_min_size(r.min, vec2(r.width(), 20.0));
        let painter = ui.painter();
        for (x, s) in [(0.0, t("Name")), (name_w, t("Location"))] {
            painter.text(
                pos2(head.left() + x + 4.0, head.center().y),
                Align2::LEFT_CENTER,
                s,
                FontId::proportional(12.0),
                INK,
            );
        }
        painter.hline(head.x_range(), head.bottom(), Stroke::new(1.0, LINE));
        let list = Rect::from_min_max(pos2(r.left(), head.bottom() + 1.0), r.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, path) in recent.iter().enumerate() {
                    let (row, resp) =
                        ui.allocate_exact_size(vec2(list.width(), ROW_H), Sense::click());
                    let name = stem(path);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                    if resp.clicked() {
                        self.launch.selected = Some(i);
                    }
                    if resp.double_clicked() {
                        *act = Some(Act::Open(path.clone()));
                    }
                    let painter = ui.painter();
                    if self.launch.selected == Some(i) {
                        painter.rect_filled(row, 0.0, Color32::from_rgb(204, 228, 247));
                    } else if resp.hovered() {
                        painter.rect_filled(row, 0.0, Color32::from_rgb(242, 246, 252));
                    }
                    painter.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
                    file_icon(
                        painter,
                        Rect::from_center_size(
                            pos2(row.left() + 35.0, row.center().y),
                            vec2(32.0, 32.0),
                        ),
                    );
                    let text = |s: &str, x: f32, w: f32, color| {
                        let mut job = egui::text::LayoutJob::single_section(
                            s.to_string(),
                            egui::TextFormat::simple(FontId::proportional(12.5), color),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                        let g = painter.layout_job(job);
                        let y = row.center().y - g.size().y / 2.0;
                        painter.galley(pos2(row.left() + x, y), g, color);
                    };
                    text(&name, 66.0, name_w - 70.0, LINK);
                    text(path, name_w + 6.0, row.width() - name_w - 12.0, LINK);
                }
            });
        });
    }

    /// Create: Blank and Convert, with the chosen one described on the right.
    fn create_tab(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        let r = ui.max_rect();
        let right = Rect::from_min_max(pos2(r.right() - RIGHT_W, r.top()), r.max);
        let painter = ui.painter().clone();
        let tiles = [(false, "Blank"), (true, "Convert")];
        for (i, (convert, label)) in tiles.into_iter().enumerate() {
            let tile = Rect::from_min_size(
                r.min + vec2(16.0 + i as f32 * 168.0, 14.0),
                vec2(158.0, 158.0),
            );
            let resp = ui.interact(
                tile,
                egui::Id::NULL.with(("create_tile", i)),
                Sense::click(),
            );
            let sel = self.launch.convert == convert;
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, sel, t(label)));
            if resp.clicked() {
                self.launch.convert = convert;
            }
            if resp.double_clicked() {
                *act = Some(Act::Create);
            }
            if sel || resp.hovered() {
                let fill = if sel {
                    Color32::from_rgb(238, 238, 238)
                } else {
                    Color32::from_rgb(246, 246, 246)
                };
                painter.rect_filled(tile, 10.0, fill);
            }
            let icon =
                Rect::from_center_size(tile.center_top() + vec2(0.0, 66.0), vec2(42.0, 60.0));
            if convert {
                convert_icon(&painter, icon);
            } else {
                page(&painter, icon, Color32::WHITE, 1.0, true);
            }
            let font = FontId::proportional(13.0);
            let at = tile.center_top() + vec2(0.0, 131.0);
            if sel {
                let g = painter.layout_no_wrap(t(label).into(), font, Color32::WHITE);
                let bg = Rect::from_center_size(at, g.size() + vec2(12.0, 6.0));
                painter.rect_filled(bg, 2.0, ACCENT);
                bold(&painter, bg.min + vec2(6.0, 3.0), g, Color32::WHITE);
            } else {
                let g = painter.layout_no_wrap(t(label).into(), font, INK);
                bold(&painter, at - g.size() / 2.0, g, INK);
            }
        }
        // The details panel.
        painter.rect_filled(right, 0.0, Color32::from_rgb(230, 230, 230));
        painter.vline(right.left(), right.y_range(), Stroke::new(1.0, LINE));
        let preview =
            Rect::from_min_size(right.min + vec2(10.0, 16.0), vec2(RIGHT_W - 20.0, 200.0));
        painter.rect_filled(preview, 0.0, Color32::from_rgb(224, 224, 224));
        painter.rect_stroke(
            preview,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(190, 190, 190)),
            StrokeKind::Inside,
        );
        let big = Rect::from_center_size(preview.center(), vec2(88.0, 125.0));
        let (title, text) = if self.launch.convert {
            convert_icon(&painter, big);
            (
                "Convert",
                "Creates an app from a CSV, tab-separated or Excel file: one field per column, one record per row.",
            )
        } else {
            page(&painter, big, Color32::WHITE, 2.0, true);
            (
                "Blank",
                "An empty app with one table and one layout, ready for your fields.",
            )
        };
        let info = Rect::from_min_max(
            pos2(right.left() + 12.0, preview.bottom() + 14.0),
            pos2(right.right() - 12.0, right.bottom() - 50.0),
        );
        let g = painter.layout_no_wrap(t(title).into(), FontId::proportional(13.0), INK);
        bold(&painter, info.min, g, INK);
        let info = info.with_min_y(info.top() + 24.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(info), |ui| {
            ui.label(egui::RichText::new(t(text)).color(INK).size(12.5));
        });
        let button = Rect::from_min_max(
            pos2(right.left() + 12.0, right.bottom() - 40.0),
            pos2(right.right() - 12.0, right.bottom() - 14.0),
        );
        let b = egui::Button::new(egui::RichText::new(t("Create")).color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(0.0);
        if ui.put(button, b).clicked() {
            *act = Some(Act::Create);
        }
    }
}

/// Favorites: "Add App", then a card per favorite file; a click opens it.
fn favorites_tab(ui: &mut egui::Ui, favorites: &[String], act: &mut Option<Act>) {
    let r = ui.max_rect();
    let size = vec2(160.0, 160.0);
    let per_row = (((r.width() - 16.0) / (size.x + 12.0)).floor() as usize).max(1);
    let painter = ui.painter().clone();
    for i in 0..=favorites.len() {
        let at = r.min
            + vec2(16.0, 12.0)
            + vec2(
                (i % per_row) as f32 * (size.x + 12.0),
                (i / per_row) as f32 * (size.y + 12.0),
            );
        let card = Rect::from_min_size(at, size);
        let id = egui::Id::NULL.with(("favorite", i));
        let (label, path) = match i.checked_sub(1).and_then(|j| favorites.get(j)) {
            None => (t("Add App").to_string(), None),
            Some(p) => (stem(p), Some(p)),
        };
        let enabled = path.is_some() || cfg!(not(target_arch = "wasm32"));
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.interact(card, id, sense);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
        let fill = if resp.hovered() && enabled {
            Color32::from_rgb(244, 244, 244)
        } else {
            Color32::WHITE
        };
        painter.rect_filled(card, 8.0, fill);
        painter.rect_stroke(
            card,
            8.0,
            Stroke::new(1.0, Color32::from_rgb(160, 160, 160)),
            StrokeKind::Inside,
        );
        let c = card.center_top() + vec2(0.0, 62.0);
        match path {
            None => {
                let s = Stroke::new(6.0, Color32::from_rgb(200, 200, 200));
                painter.line_segment([c - vec2(15.0, 0.0), c + vec2(15.0, 0.0)], s);
                painter.line_segment([c - vec2(0.0, 15.0), c + vec2(0.0, 15.0)], s);
            }
            Some(p) => {
                file_icon(&painter, Rect::from_center_size(c, vec2(48.0, 48.0)));
                resp.clone().on_hover_text(p);
            }
        }
        let mut job = egui::text::LayoutJob::single_section(
            label.clone(),
            egui::TextFormat::simple(FontId::proportional(13.0), INK),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(size.x - 16.0);
        let g = painter.layout_job(job);
        bold(
            &painter,
            card.center_top() + vec2(-g.size().x / 2.0, 117.0),
            g,
            INK,
        );
        if resp.clicked() {
            *act = Some(match path {
                None => Act::AddFavorite,
                Some(p) => Act::Open(p.clone()),
            });
        }
        if path.is_some() {
            resp.context_menu(|ui| {
                if ui.button(t("Remove from Favorites")).clicked() {
                    *act = Some(Act::RemoveFavorite(i - 1));
                    ui.close();
                }
            });
        }
    }
}

/// Bold text: egui's fonts have no bold face, so the text is drawn twice.
fn bold(p: &egui::Painter, at: Pos2, g: std::sync::Arc<egui::Galley>, color: Color32) {
    p.galley(at, g.clone(), color);
    p.galley(at + vec2(0.6, 0.0), g, color);
}

/// A page with a folded corner (and a "+" on it).
fn page(p: &egui::Painter, r: Rect, fill: Color32, width: f32, plus: bool) {
    let f = r.width() * 0.3;
    let pts = vec![
        r.left_top(),
        pos2(r.right() - f, r.top()),
        pos2(r.right(), r.top() + f),
        r.right_bottom(),
        r.left_bottom(),
    ];
    p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    let s = Stroke::new(width, ICON);
    p.add(Shape::closed_line(pts, s));
    p.add(Shape::line(
        vec![
            pos2(r.right() - f, r.top()),
            pos2(r.right() - f, r.top() + f),
            pos2(r.right(), r.top() + f),
        ],
        s,
    ));
    if plus {
        let c = r.center() + vec2(0.0, f * 0.15);
        let h = r.width() * 0.18;
        p.line_segment([c - vec2(h, 0.0), c + vec2(h, 0.0)], s);
        p.line_segment([c - vec2(0.0, h), c + vec2(0.0, h)], s);
    }
}

/// A file of the lists: a page holding a stack of record lines.
fn file_icon(p: &egui::Painter, r: Rect) {
    let tile = Rect::from_center_size(r.center(), r.size() * 0.95);
    p.rect_filled(tile, 3.0, Color32::from_rgb(226, 218, 244));
    let pg = Rect::from_center_size(
        tile.center(),
        vec2(tile.width() * 0.5, tile.height() * 0.62),
    );
    page(p, pg, Color32::WHITE, 1.0, false);
    let s = Stroke::new(1.0, Color32::from_rgb(112, 64, 170));
    for k in 0..3 {
        let y = pg.top() + pg.height() * (0.45 + 0.17 * k as f32);
        p.line_segment([pos2(pg.left() + 3.0, y), pos2(pg.right() - 3.0, y)], s);
    }
}

/// Convert: a page in front of another, with a turning arrow.
fn convert_icon(p: &egui::Painter, r: Rect) {
    let w = r.width() * 0.78;
    let h = r.height() * 0.82;
    let back = Rect::from_min_size(pos2(r.right() - w, r.top()), vec2(w, h));
    let front = Rect::from_min_size(pos2(r.left(), r.bottom() - h), vec2(w, h));
    page(p, back, Color32::from_rgb(222, 222, 222), 1.0, false);
    page(p, front, Color32::WHITE, 1.0, false);
    let s = Stroke::new(1.0, ICON);
    let c = front.center() + vec2(front.width() * 0.05, front.height() * 0.05);
    let rad = front.width() * 0.28;
    let arc: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = std::f32::consts::PI * (-0.5 + k as f32 / 12.0 * 1.0);
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    let end = pos2(c.x - rad * 0.9, c.y + rad);
    let mut line = arc;
    line.push(end);
    p.add(Shape::line(line, s));
    let a = rad * 0.35;
    p.line_segment([end, end + vec2(a, -a)], s);
    p.line_segment([end, end + vec2(a, a)], s);
}

/// Favorites: a star with a small "+" box.
fn star_icon(p: &egui::Painter, c: Pos2) {
    let pts: Vec<Pos2> = (0..10)
        .map(|k| {
            let r = if k % 2 == 0 { 23.0 } else { 10.0 };
            let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::convex_polygon(
        pts.iter().skip(1).step_by(2).copied().collect(),
        Color32::from_rgb(228, 228, 228),
        Stroke::NONE,
    ));
    for k in (0..10).step_by(2) {
        let tri = vec![pts[(k + 9) % 10], pts[k], pts[k + 1]];
        p.add(Shape::convex_polygon(
            tri,
            Color32::from_rgb(228, 228, 228),
            Stroke::NONE,
        ));
    }
    p.add(Shape::closed_line(pts, Stroke::new(1.0, ICON)));
    let b = Rect::from_center_size(c + vec2(12.0, 16.0), vec2(11.0, 11.0));
    p.rect_filled(b, 1.0, Color32::from_rgb(245, 245, 245));
    p.rect_stroke(b, 1.0, Stroke::new(1.0, ICON), StrokeKind::Inside);
    let s = Stroke::new(1.0, INK);
    p.line_segment(
        [b.center() - vec2(3.0, 0.0), b.center() + vec2(3.0, 0.0)],
        s,
    );
    p.line_segment(
        [b.center() - vec2(0.0, 3.0), b.center() + vec2(0.0, 3.0)],
        s,
    );
}

/// Recent: a clock.
fn clock_icon(p: &egui::Painter, c: Pos2) {
    p.circle(
        c,
        20.0,
        Color32::from_rgb(232, 232, 232),
        Stroke::new(1.5, ICON),
    );
    p.circle_stroke(c, 16.0, Stroke::new(1.0, ICON));
    let s = Stroke::new(1.5, INK);
    p.line_segment([c, c + vec2(0.0, -11.0)], s);
    p.line_segment([c, c + vec2(7.0, 6.0)], s);
}

/// Create: a pencil crossing a brush.
fn create_icon(p: &egui::Painter, c: Pos2) {
    let s = Stroke::new(1.0, ICON);
    // A bar from `a` to `b`, `w` wide.
    let bar = |a: Pos2, b: Pos2, w: f32, fill: Color32| {
        let n = (b - a).normalized().rot90() * (w / 2.0);
        p.add(Shape::convex_polygon(
            vec![a + n, b + n, b - n, a - n],
            fill,
            s,
        ));
    };
    // The pencil, top left to bottom right, its point at the bottom.
    let (a, b) = (c + vec2(-17.0, -17.0), c + vec2(10.0, 10.0));
    bar(a, b, 6.0, Color32::from_rgb(236, 236, 236));
    let d = (b - a).normalized();
    let n = d.rot90() * 3.0;
    p.add(Shape::convex_polygon(
        vec![b + n, b + d * 8.0, b - n],
        Color32::WHITE,
        s,
    ));
    // The brush, top right to bottom left: a thin handle, then the bristles.
    let (a, b) = (c + vec2(17.0, -17.0), c + vec2(-6.0, 6.0));
    bar(a, b, 4.0, Color32::from_rgb(236, 236, 236));
    let d = (b - a).normalized();
    let n = d.rot90();
    let tip = b + d * 13.0;
    p.add(Shape::convex_polygon(
        vec![
            b + n * 3.0,
            tip + n * 1.0 + d.rot90() * 0.0,
            tip - n * 4.0,
            b - n * 3.0,
        ],
        Color32::from_rgb(200, 200, 200),
        s,
    ));
}

/// File name without folder or extension, for Windows and Unix paths alike
/// (the preferences may come from either).
fn stem(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}
