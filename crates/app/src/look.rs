//! Resolves FileMaker theme styles into what egui needs to paint an object.

use eframe::egui::{self, Color32, CornerRadius, Rect, Stroke, StrokeKind, Vec2};
use fmp12::theme::{INNER_BORDER, NORMAL, ObjectStyle, PropsExt, SELF, TEXT, Theme, Value, prop};

/// Paint attributes of one layout object (or part, or layout background).
#[derive(Debug, Clone)]
pub struct Look {
    pub background: Option<Color32>,
    /// Linear gradient over the background: start and end as fractions of
    /// the box, and the colour stops.
    pub gradient: Option<Gradient>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub start: Vec2,
    pub end: Vec2,
    pub stops: Vec<(f32, Color32)>,
}

impl Gradient {
    fn from_theme(g: &fmp12::theme::Gradient) -> Self {
        // Points are percentages of the box (lengths in points kept as is).
        let f = |(x, y): (fmp12::theme::Length, fmp12::theme::Length)| {
            Vec2::new(x.pt(0.0, 1.0) as f32, y.pt(0.0, 1.0) as f32)
        };
        Gradient {
            start: f(g.start),
            end: f(g.end),
            stops: g.stops.iter().map(|(p, c)| (*p, color(*c))).collect(),
        }
    }

    /// Colour at `p` (fractions of the box).
    fn at(&self, p: Vec2) -> Color32 {
        let d = self.end - self.start;
        let t = if d.length_sq() > 0.0 {
            ((p - self.start).dot(d) / d.length_sq()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut prev = self.stops[0];
        for &s in &self.stops {
            if t <= s.0 {
                let span = s.0 - prev.0;
                let k = if span > 0.0 { (t - prev.0) / span } else { 1.0 };
                return prev.1.lerp_to_gamma(s.1, k);
            }
            prev = s;
        }
        prev.1
    }

    /// Fills `r` (corners rounded by `radius`) as a triangle fan whose
    /// vertex colours follow the gradient. ponytail: colours interpolate
    /// linearly between vertices, exact for two stops only.
    fn fill(&self, painter: &egui::Painter, r: Rect, radius: f32) {
        let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0);
        let mut pts = Vec::new();
        let corners = [
            (r.right_top() + Vec2::new(-rad, rad), -90.0f32),
            (r.right_bottom() + Vec2::new(-rad, -rad), 0.0),
            (r.left_bottom() + Vec2::new(rad, -rad), 90.0),
            (r.left_top() + Vec2::new(rad, rad), 180.0),
        ];
        for (c, a0) in corners {
            for i in 0..=8 {
                let a = (a0 + i as f32 * 11.25).to_radians();
                pts.push(c + rad * Vec2::new(a.cos(), a.sin()));
            }
        }
        let frac =
            |p: egui::Pos2| Vec2::new((p.x - r.left()) / r.width(), (p.y - r.top()) / r.height());
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(r.center(), self.at(frac(r.center())));
        for p in &pts {
            mesh.colored_vertex(*p, self.at(frac(*p)));
        }
        let n = pts.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

impl Default for Look {
    fn default() -> Self {
        Look {
            background: None,
            gradient: None,
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
    l.gradient = p
        .get(&prop::BACKGROUND_IMAGE)
        .and_then(Value::gradient)
        .map(|g| Gradient::from_theme(&g));
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
    // Corner radii are (horizontal, vertical) pairs; the first corner stands
    // for all four.
    l.radius = match p.get(&prop::BORDER_RADIUS) {
        Some(Value::Pair(x, _) | Value::Length(x)) => x.pt(font_size, 0.0) as f32,
        _ => 0.0,
    };
    // Text colour may live on the `.text` element or on `.self`; the
    // object's own (local) colour wins over the theme's.
    let text = th.resolve(style, TEXT, NORMAL);
    let local = |e| {
        style
            .local
            .get(&(e, NORMAL))
            .and_then(|p| p.color(prop::COLOR))
    };
    if let Some(c) = local(TEXT)
        .or_else(|| local(SELF))
        .or_else(|| text.color(prop::COLOR))
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

    /// Paints an object's shape: an ellipse for ovals (kind 7), else
    /// [`Look::paint_box`].
    pub fn paint_shape(&self, painter: &egui::Painter, r: Rect, kind: u64) {
        if kind != 7 {
            return self.paint_box(painter, r);
        }
        let (c, rad) = (r.center(), r.size() / 2.0);
        if let Some(bg) = self.background {
            painter.add(egui::Shape::ellipse_filled(c, rad, bg));
        }
        let (w, col) = self.borders[0];
        if w > 0.0 {
            painter.add(egui::Shape::ellipse_stroke(c, rad, Stroke::new(w, col)));
        }
    }

    /// Paints background and borders.
    pub fn paint_box(&self, painter: &egui::Painter, r: Rect) {
        let rad = CornerRadius::same(self.radius.min(255.0) as u8);
        if let Some(bg) = self.background {
            painter.rect_filled(r, rad, bg);
        }
        if let Some(g) = &self.gradient {
            g.fill(painter, r, self.radius);
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
        let mut job = egui::text::LayoutJob::simple(
            text.replace('\r', "\n"),
            self.font(),
            self.color,
            f32::INFINITY,
        );
        job.halign = anchor.x();
        let galley = painter.layout_job(job);
        let at = anchor.anchor_size(egui::pos2(x, y), galley.size());
        painter
            .with_clip_rect(r)
            .galley(at.min - galley.rect.min.to_vec2(), galley, self.color);
    }
}
