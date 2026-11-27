//! Charts in Browse and Preview: the chart's values and shapes come from
//! `print::chart` (shared with the PDF output); this paints them with egui.

use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use fmp12::layouts::{Layout, LayoutObject};
use print::chart::Shape;

use crate::Doc;

/// The file's charts, decoded once per opened file (kept in egui's memory).
fn charts(ctx: &egui::Context, doc: &Doc) -> Arc<print::Charts> {
    let id = egui::Id::NULL.with(("charts", &doc.name, doc.file.bytes().len()));
    if let Some(c) = ctx.data(|d| d.get_temp::<Arc<print::Charts>>(id)) {
        return c;
    }
    let c = Arc::new(fmp12::charts::charts(&doc.file).unwrap_or_default());
    ctx.data_mut(|d| d.insert_temp(id, c.clone()));
    c
}

/// Draws `o` if it is a chart: its values from `found` (the found set) or
/// from `record`. Returns `false` for other objects.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    painter: &egui::Painter,
    doc: &Doc,
    layout: &Layout,
    o: &LayoutObject,
    r: Rect,
    found: &[usize],
    record: Option<usize>,
) -> bool {
    if o.kind != 13 {
        return false;
    }
    let all = charts(ctx, doc);
    let Some(c) = all.get(&(layout.id, o.id)) else {
        return false;
    };
    let to = (layout.table_occurrence & 0xFFF) as u32;
    let d = print::chart::data(&doc.db, c, to, found, record);
    let rect = (
        r.left() as f64,
        r.top() as f64,
        r.width() as f64,
        r.height() as f64,
    );
    paint(
        &painter.with_clip_rect(r),
        &print::chart::shapes(c, &d, rect),
    );
    true
}

fn color(c: [f32; 4]) -> Color32 {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

fn pos(p: (f64, f64)) -> Pos2 {
    Pos2::new(p.0 as f32, p.1 as f32)
}

pub fn paint(painter: &egui::Painter, shapes: &[Shape]) {
    for s in shapes {
        match s {
            Shape::Rect {
                x,
                y,
                w,
                h,
                fill,
                stroke,
            } => {
                let r = Rect::from_min_size(pos((*x, *y)), egui::vec2(*w as f32, *h as f32));
                painter.rect_filled(r, 0.0, color(*fill));
                if let Some(s) = stroke {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(0.75, color(*s)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            Shape::Line {
                a,
                b,
                width,
                color: c,
            } => {
                painter.line_segment([pos(*a), pos(*b)], Stroke::new(*width as f32, color(*c)));
            }
            Shape::Polygon { points, fill } => {
                // Star-shaped from the first point: a triangle fan.
                let mut mesh = egui::Mesh::default();
                for p in points {
                    mesh.colored_vertex(pos(*p), color(*fill));
                }
                for i in 1..points.len().saturating_sub(1) as u32 {
                    mesh.add_triangle(0, i, i + 1);
                }
                painter.add(mesh);
            }
            Shape::Circle { c, r, fill, stroke } => {
                painter.circle(
                    pos(*c),
                    *r as f32,
                    color(*fill),
                    Stroke::new(1.0, color(*stroke)),
                );
            }
            Shape::Text {
                x,
                y,
                size,
                color: c,
                text,
            } => {
                // `y` is the baseline.
                painter.text(
                    pos((*x, *y + *size as f64 * 0.22)),
                    egui::Align2::LEFT_BOTTOM,
                    text,
                    egui::FontId::proportional(*size),
                    color(*c),
                );
            }
        }
    }
}
