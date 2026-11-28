//! Resolves FileMaker theme styles into what egui needs to paint an object.

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Color32, CornerRadius, Rect, Stroke, StrokeKind, Vec2};
use fmp12::layouts::CharStyle;
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
    /// Borders of the `.inner_border` element, drawn inside those (fields
    /// of layouts converted from older versions have theirs there).
    pub inner_borders: [(f32, Color32); 4],
    pub radius: f32,
    pub color: Color32,
    pub font_size: f32,
    /// 0 left, 1 centre, 2 right.
    pub align: u8,
    /// 0 top, 1 centre, 2 bottom.
    pub valign: u8,
    /// Padding: top, right, bottom, left.
    pub padding: [f32; 4],
    /// Bold text (`font-weight: bold`).
    pub bold: bool,
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
            inner_borders: [(0.0, Color32::TRANSPARENT); 4],
            radius: 0.0,
            color: Color32::BLACK,
            font_size: 12.0,
            align: 0,
            valign: 1,
            padding: [2.0; 4],
            bold: false,
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
        let side = |p: &fmp12::theme::Props| {
            // Border style 1 = none.
            let none = p.enum_value(prop::BORDER_STYLE + i) == Some(1);
            let w = p
                .length(prop::BORDER_WIDTH + i)
                .map_or(0.0, |x| x.pt(font_size, 0.0) as f32);
            let c = p
                .color(prop::BORDER_COLOR + i)
                .map_or(Color32::TRANSPARENT, color);
            if none { (0.0, c) } else { (w, c) }
        };
        l.borders[i as usize] = side(&p);
        l.inner_borders[i as usize] = side(&inner);
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
    l.bold = [&p, &text]
        .iter()
        .any(|x| x.enum_value(prop::FONT_WEIGHT) == Some(2));
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
        stroke_sides(painter, r, rad, self.borders);
        let [t, ri, b, le] = self.borders.map(|s| s.0);
        let inner = Rect::from_min_max(r.min + Vec2::new(le, t), r.max - Vec2::new(ri, b));
        stroke_sides(painter, inner, rad, self.inner_borders);
    }

    /// Frameless single-line editor styled like the field.
    pub fn edit<'a>(&self, s: &'a mut String) -> egui::TextEdit<'a> {
        let h = [egui::Align::Min, egui::Align::Center, egui::Align::Max][self.align as usize];
        egui::TextEdit::singleline(s)
            .frame(egui::Frame::NONE)
            // The padding is the field's own (`content_rect`).
            .margin(egui::Margin::ZERO)
            .font(self.font())
            .text_color(self.color)
            .align(egui::Align2([h, egui::Align::Center]))
    }

    /// Where an editor goes for a field whose content area is `inner`:
    /// at least one text row high, centred on it (a short field's text
    /// overflows evenly, as FileMaker draws it, instead of downwards).
    pub fn edit_rect(&self, ctx: &egui::Context, inner: Rect) -> Rect {
        let row = ctx.fonts_mut(|f| f.row_height(&self.font()));
        Rect::from_center_size(
            inner.center(),
            Vec2::new(inner.width(), inner.height().max(row)),
        )
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
        let p = painter.with_clip_rect(r);
        let pos = at.min - galley.rect.min.to_vec2();
        // egui's fonts have no bold face: bold text is drawn twice.
        if self.bold {
            p.galley(pos + Vec2::new(0.6, 0.0), galley.clone(), self.color);
        }
        p.galley(pos, galley, self.color);
    }

    /// Paints a text object's text with its style runs and paragraph
    /// alignment (FORMAT.md §10), each piece of stored text passed through
    /// `f` (merge fields resolved, or spelled by name in Layout mode).
    pub fn paint_styled(
        &self,
        painter: &egui::Painter,
        r: Rect,
        o: &fmp12::layouts::LayoutObject,
        f: impl Fn(&str) -> String,
    ) {
        let text = o.text.as_deref().unwrap_or("");
        let runs = o.char_runs();
        let aligns = o.paragraph_aligns();
        if runs.is_empty() && aligns.iter().all(|a| *a == 0) {
            return self.paint_text(painter, r, &f(text));
        }
        // Paragraphs of (text, style) pieces.
        let mut paras: Vec<Vec<(String, CharStyle)>> = vec![Vec::new()];
        for (piece, style) in fmp12::layouts::styled_pieces(text, &runs) {
            for (i, line) in f(piece).split(['\r', '\n']).enumerate() {
                if i > 0 {
                    paras.push(Vec::new());
                }
                if let Some(p) = paras.last_mut() {
                    p.push((line.to_string(), style));
                }
            }
        }
        let jobs: Vec<(LayoutJob, LayoutJob, u8)> = paras
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // FileMaker: 1 left, 2 centre, 3 right, 4 full (drawn left).
                let align = match aligns.get(i).copied().unwrap_or(0) {
                    0 => self.align,
                    2 => 1,
                    3 => 2,
                    _ => 0,
                };
                let (job, bold) = self.styled_job(p);
                (job, bold, align)
            })
            .collect();
        let galleys: Vec<_> = jobs
            .into_iter()
            .map(|(j, b, a)| (painter.layout_job(j), painter.layout_job(b), a))
            .collect();
        let c = self.content_rect(r);
        let height: f32 = galleys.iter().map(|g| g.0.size().y).sum();
        let mut y = match self.valign {
            0 => c.top(),
            1 => c.center().y - height / 2.0,
            _ => c.bottom() - height,
        };
        let p = painter.with_clip_rect(r);
        for (g, bold, align) in galleys {
            let w = g.size().x;
            let x = [c.left(), c.center().x - w / 2.0, c.right() - w][align as usize];
            let pos = egui::pos2(x, y) - g.rect.min.to_vec2();
            // egui's fonts have no bold face: bold pieces are drawn twice.
            p.galley(pos + Vec2::new(0.6, 0.0), bold, self.color);
            y += g.size().y;
            p.galley(pos, g, self.color);
        }
    }

    /// The layout job of one paragraph, and a copy where only its bold
    /// pieces are visible.
    fn styled_job(&self, pieces: &[(String, CharStyle)]) -> (LayoutJob, LayoutJob) {
        use fmp12::layouts::face;
        let mut job = LayoutJob::default();
        let mut bold = LayoutJob::default();
        for (s, st) in pieces {
            let f = st.face.unwrap_or(if self.bold { face::BOLD } else { 0 });
            let on = |bit: u16| f & bit != 0;
            let size = st.size.map_or(self.font_size, f32::from).max(4.0);
            let color = st.color.map_or(self.color, |[r, g, b, a]| {
                Color32::from_rgba_unmultiplied(r, g, b, a)
            });
            let line = Stroke::new((size / 14.0).max(1.0), color);
            let script = on(face::SUPERSCRIPT) || on(face::SUBSCRIPT);
            let mut fmt = egui::TextFormat {
                font_id: egui::FontId::proportional(if script { size * 0.6 } else { size }),
                color,
                italics: on(face::ITALIC),
                underline: if on(face::UNDERLINE | face::WORD_UNDERLINE | face::DOUBLE_UNDERLINE) {
                    line
                } else {
                    Stroke::NONE
                },
                strikethrough: if on(face::STRIKETHROUGH) {
                    line
                } else {
                    Stroke::NONE
                },
                background: if on(face::HIGHLIGHT) {
                    Color32::YELLOW
                } else {
                    Color32::TRANSPARENT
                },
                line_height: Some(size * 1.2),
                ..Default::default()
            };
            // Small text on the row's top reads as superscript (egui
            // aligns pieces on their bottom otherwise).
            if on(face::SUPERSCRIPT) {
                fmt.valign = egui::Align::Min;
            }
            let s = if on(face::UPPERCASE) {
                s.to_uppercase()
            } else if on(face::LOWERCASE) {
                s.to_lowercase()
            } else {
                s.clone()
            };
            job.append(&s, 0.0, fmt.clone());
            if !on(face::BOLD) {
                fmt.color = Color32::TRANSPARENT;
                fmt.underline = Stroke::NONE;
                fmt.strikethrough = Stroke::NONE;
                fmt.background = Color32::TRANSPARENT;
            }
            bold.append(&s, 0.0, fmt);
        }
        job.wrap.max_width = f32::INFINITY;
        bold.wrap.max_width = f32::INFINITY;
        (job, bold)
    }
}

/// Strokes the sides of `r` inside it: top, right, bottom, left.
fn stroke_sides(painter: &egui::Painter, r: Rect, rad: CornerRadius, sides: [(f32, Color32); 4]) {
    let [t, ri, b, le] = sides;
    if sides.iter().all(|s| s == &t) && t.0 > 0.0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use fmp12::theme::{Length, Rgba, Unit};

    /// A field as layouts converted from older versions have it: bold
    /// text, a white outset border on `.self` and its visible border on
    /// `.inner_border`.
    #[test]
    fn bold_and_inner_borders() {
        let mut style = ObjectStyle::default();
        let black = Value::Color(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        });
        let pt = Value::Length(Length {
            value: 1.0,
            unit: Unit::Pt,
        });
        let own = style.local.entry((SELF, NORMAL)).or_default();
        own.insert(prop::FONT_WEIGHT, Value::Enum(2));
        let inner = style.local.entry((INNER_BORDER, NORMAL)).or_default();
        for i in 0..4 {
            inner.insert(prop::BORDER_STYLE + i, Value::Enum(5));
            inner.insert(prop::BORDER_WIDTH + i, pt.clone());
            inner.insert(prop::BORDER_COLOR + i, black.clone());
        }
        let l = resolve(Some(&Theme::default()), &style);
        assert!(l.bold);
        assert_eq!(l.borders[0].0, 0.0);
        assert_eq!(l.inner_borders, [(1.0, Color32::BLACK); 4]);
    }
}
