//! Resolves FileMaker theme styles into what egui needs to paint an object.

use eframe::egui::{self, Color32, CornerRadius, Rect, Stroke, StrokeKind, Vec2};
use fmp12::theme::{INNER_BORDER, NORMAL, ObjectStyle, PropsExt, SELF, TEXT, Theme, prop};

/// Paint attributes of one layout object (or part, or layout background).
#[derive(Debug, Clone)]
pub struct Look {
    pub background: Option<Color32>,
    /// Border width and colour per side: top, right, bottom, left.
    pub borders: [(f32, Color32); 4],
    pub radius: f32,
    pub color: Color32,
    pub font_size: f32,
    /// 0 left, 1 centre, 2 right.
    pub align: u8,
    /// 0 top, 1 centre, 2 bottom.
    pub valign: u8,
    /// Padding: top, right, bottom, left.
    pub padding: [f32; 4],
}

impl Default for Look {
    fn default() -> Self {
        Look {
            background: None,
            borders: [(0.0, Color32::TRANSPARENT); 4],
            radius: 0.0,
            color: Color32::BLACK,
            font_size: 12.0,
            align: 0,
            valign: 1,
            padding: [2.0; 4],
        }
    }
}

fn color(c: fmp12::theme::Rgba) -> Color32 {
    let u = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(u(c.r), u(c.g), u(c.b), u(c.a))
}

/// Effective look of an object style under `theme`, in the normal state.
pub fn resolve(theme: Option<&Theme>, style: &ObjectStyle) -> Look {
    let Some(th) = theme else {
        return Look::default();
    };
    let p = th.resolve(style, SELF, NORMAL);
    let inner = th.resolve(style, INNER_BORDER, NORMAL);
    let mut l = Look::default();
    let font_size = p.length(prop::FONT_SIZE).map_or(12.0, |x| x.pt(12.0, 12.0));
    l.font_size = font_size as f32;
    l.background = p
        .color(prop::BACKGROUND_COLOR)
        .map(color)
        .filter(|c| c.a() > 0);
    for i in 0..4u32 {
        // Border style 1 = none.
        let none = p.enum_value(prop::BORDER_STYLE + i) == Some(1);
        let w = p
            .length(prop::BORDER_WIDTH + i)
            .map_or(0.0, |x| x.pt(font_size, 0.0) as f32);
        let c = p
            .color(prop::BORDER_COLOR + i)
            .map_or(Color32::TRANSPARENT, color);
        l.borders[i as usize] = if none { (0.0, c) } else { (w, c) };
        l.padding[i as usize] = inner
            .length(prop::PADDING + i)
            .map_or(2.0, |x| x.pt(font_size, 0.0) as f32);
    }
    l.radius = p
        .length(prop::BORDER_RADIUS)
        .map_or(0.0, |x| x.pt(font_size, 0.0) as f32);
    // Text colour may live on the `.text` element or on `.self`.
    let text = th.resolve(style, TEXT, NORMAL);
    if let Some(c) = text
        .color(prop::COLOR)
        .or_else(|| p.color(prop::COLOR))
        .filter(|c| c.a > 0.0)
    {
        l.color = color(c);
    }
    l.align = match p.enum_value(prop::TEXT_ALIGN) {
        Some(2) => 1,
        Some(3) => 2,
        _ => 0,
    };
    l.valign = p
        .enum_value(prop::VERTICAL_ALIGN)
        .map_or(1, |v| v.clamp(0, 2) as u8);
    l
}

impl Look {
    pub fn font(&self) -> egui::FontId {
        egui::FontId::proportional(self.font_size.max(6.0))
    }

    /// Paints background and borders.
    pub fn paint_box(&self, painter: &egui::Painter, r: Rect) {
        let rad = CornerRadius::same(self.radius.min(255.0) as u8);
        if let Some(bg) = self.background {
            painter.rect_filled(r, rad, bg);
        }
        let [t, ri, b, le] = self.borders;
        if [t, ri, b, le].iter().all(|s| s == &t) && t.0 > 0.0 {
            painter.rect_stroke(r, rad, Stroke::new(t.0, t.1), StrokeKind::Inside);
            return;
        }
        let side = |w: (f32, Color32), a: egui::Pos2, z: egui::Pos2| {
            if w.0 > 0.0 {
                painter.line_segment([a, z], Stroke::new(w.0, w.1));
            }
        };
        side(t, r.left_top(), r.right_top());
        side(ri, r.right_top(), r.right_bottom());
        side(b, r.left_bottom(), r.right_bottom());
        side(le, r.left_top(), r.left_bottom());
    }

    /// Frameless single-line editor styled like the field.
    pub fn edit<'a>(&self, s: &'a mut String) -> egui::TextEdit<'a> {
        let h = [egui::Align::Min, egui::Align::Center, egui::Align::Max][self.align as usize];
        egui::TextEdit::singleline(s)
            .frame(egui::Frame::NONE)
            .font(self.font())
            .text_color(self.color)
            .align(egui::Align2([h, egui::Align::Center]))
    }

    /// Area inside the padding.
    pub fn content_rect(&self, r: Rect) -> Rect {
        let [t, ri, b, le] = self.padding;
        Rect::from_min_max(r.min + Vec2::new(le, t), r.max - Vec2::new(ri, b))
    }

    /// Paints text aligned inside the content area.
    pub fn paint_text(&self, painter: &egui::Painter, r: Rect, text: &str) {
        let c = self.content_rect(r);
        let x = [c.left(), c.center().x, c.right()][self.align as usize];
        let y = [c.top(), c.center().y, c.bottom()][self.valign as usize];
        let anchor = egui::Align2([
            [egui::Align::Min, egui::Align::Center, egui::Align::Max][self.align as usize],
            [egui::Align::Min, egui::Align::Center, egui::Align::Max][self.valign as usize],
        ]);
        painter
            .with_clip_rect(r)
            .text(egui::pos2(x, y), anchor, text, self.font(), self.color);
    }
}
