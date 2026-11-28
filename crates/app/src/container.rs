//! Container fields in Browse mode: pictures (PNG, JPEG), other files as an
//! icon and a name; Insert Picture/File and Export Field Contents.

use calc::Value;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind};
use engine::Database;

use crate::i18n::tr;
use crate::look::Look;

/// (table, record, field, repetition) of a container on screen.
pub type Target = (u32, usize, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Insert,
    Export,
}

fn active_id() -> egui::Id {
    egui::Id::NULL.with("basecraft-active-container")
}

/// The container field last clicked (Insert › Picture or File… fills it).
pub fn active(ctx: &egui::Context) -> Option<Target> {
    ctx.data(|d| d.get_temp(active_id()))
}

/// Draws a container; returns what its context menu asked for.
pub fn draw(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    lk: &Look,
    r: Rect,
    v: &Value,
    target: Target,
) -> Option<Action> {
    let resp = ui.interact(r, ui.scope_id().with(("container", target)), Sense::click());
    if resp.clicked() || resp.secondary_clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(active_id(), target));
    }
    if let Value::Container(c) = v {
        let name = if c.path.is_empty() {
            c.name.as_str()
        } else {
            fmp12::containers::ref_name(&c.path)
        };
        match texture(ui.ctx(), &c.data) {
            Some(tex) => {
                // Reduced to fit, centred, never enlarged.
                let size = tex.size_vec2();
                let scale = (r.width() / size.x).min(r.height() / size.y).min(1.0);
                let img = Rect::from_center_size(r.center(), size * scale);
                painter.image(
                    tex.id(),
                    img,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None if !v.is_empty() => {
                painter.text(
                    r.center_top() + egui::vec2(0.0, 2.0),
                    egui::Align2::CENTER_TOP,
                    "📄",
                    egui::FontId::proportional((r.height() * 0.5).clamp(10.0, 32.0)),
                    Color32::from_gray(90),
                );
                lk.paint_text(painter, r.with_min_y(r.center().y), name);
            }
            None => {}
        }
    }
    if active(ui.ctx()) == Some(target) {
        let blue = Stroke::new(1.5, Color32::from_rgb(80, 130, 220));
        painter.rect_stroke(r, 0.0, blue, StrokeKind::Inside);
    }
    let mut action = None;
    resp.context_menu(|ui| {
        if ui.button(tr("Insert Picture or File…")).clicked() {
            action = Some(Action::Insert);
            ui.close();
        }
        let export = egui::Button::new(tr("Export Field Contents…"));
        if ui.add_enabled(!v.is_empty(), export).clicked() {
            action = Some(Action::Export);
            ui.close();
        }
    });
    action
}

/// A graphic layout object's picture, scaled to the object's bounds.
pub fn picture(ctx: &egui::Context, painter: &egui::Painter, r: Rect, data: &[u8]) {
    if let Some(tex) = texture(ctx, data) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex.id(), r, uv, Color32::WHITE);
    }
}

/// Picture data egui can show (PNG, JPEG), decoded once per content.
fn texture(ctx: &egui::Context, data: &[u8]) -> Option<egui::TextureHandle> {
    let tag = fmp12::containers::image_tag(data)?;
    if &tag == b"GIFf" {
        return None; // ponytail: no GIF decoder in the dependency tree.
    }
    let id = egui::Id::NULL.with(("basecraft-container-image", data));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let img = image::load_from_memory(data).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    let tex = ctx.load_texture("container", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Runs Insert Picture/File (a file picked from disk goes into the
/// container) or Export Field Contents (the container's file saved to
/// disk). Returns whether the record changed.
pub fn perform(db: &mut Database, action: Action, (t, rec, field, rep): Target) -> bool {
    match action {
        Action::Insert => match pick_file(false) {
            Some((name, data)) => {
                db.insert_file(t, rec, field, rep, &name, data);
                true
            }
            None => false,
        },
        Action::Export => {
            if let Value::Container(c) = db.value_rep(t, rec, field, rep)
                && !c.data.is_empty()
            {
                save_file(&c.name, &c.data);
            }
            false
        }
    }
}

/// A file chosen by the user, as (name, bytes).
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_file(picture: bool) -> Option<(String, Vec<u8>)> {
    let mut d = rfd::FileDialog::new();
    if picture {
        d = d.add_filter(tr("Pictures"), &["png", "jpg", "jpeg", "gif"]);
    }
    let p = d.pick_file()?;
    let data = std::fs::read(&p).ok()?;
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some((name, data))
}

/// Saves `data` where the user says, proposing `name`.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_file(name: &str, data: &[u8]) -> bool {
    rfd::FileDialog::new()
        .set_file_name(name)
        .save_file()
        .is_some_and(|p| std::fs::write(p, data).is_ok())
}

// ponytail: the browser build has no synchronous file dialogs.
#[cfg(target_arch = "wasm32")]
pub fn pick_file(_: bool) -> Option<(String, Vec<u8>)> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn save_file(_: &str, _: &[u8]) -> bool {
    false
}
