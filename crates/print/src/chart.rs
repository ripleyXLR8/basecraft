//! Charts: their values from the database ([`data`]) and their drawing as
//! a list of shapes ([`shapes`]) that the app (egui) and the PDF writer
//! both paint, so Browse, Preview and PDF look the same.
//!
//! The look follows FileMaker Pro 26's default chart style ("Solid - Flat",
//! colour scheme Earth, transparent background), measured on its Browse
//! mode: title on top, legend under it, value labels on both sides of the
//! plot, a single series coloured per category, several series one colour
//! each.

use engine::Database;
use fmp12::charts::{Chart, Kind, Source};

use crate::look::Color;
use crate::pdf::text_width;

/// A chart's values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Data {
    pub title: String,
    pub x_title: String,
    pub y_title: String,
    /// X values as text (one per point).
    pub categories: Vec<String>,
    /// (name, values) per Y series.
    pub series: Vec<(String, Vec<f64>)>,
    /// Bubble sizes.
    pub sizes: Vec<f64>,
}

fn number(v: &calc::Value) -> f64 {
    v.to_num()
        .map(|n| calc::num::to_f64(&n))
        .filter(|f| f.is_finite())
        .unwrap_or(0.0)
}

fn lines(s: &str) -> Vec<String> {
    s.split(['\r', '\n']).map(str::to_string).collect()
}

/// Evaluates the chart's calculations: per record of `found` (found set),
/// per return-separated value of `record` (current record), or per related
/// record of `record` (related records). `to` is the layout's occurrence.
pub fn data(db: &Database, c: &Chart, to: u32, found: &[usize], record: Option<usize>) -> Data {
    let table = db.table_of(to).map_or(0, |t| t.id);
    let one = |t: &Option<Vec<u8>>| {
        t.as_ref()
            .map(|t| db.eval_tokens(t, table, to, record).to_text())
            .unwrap_or_default()
    };
    let mut d = Data {
        title: one(&c.title),
        x_title: one(&c.x_title),
        y_title: one(&c.y_title),
        ..Data::default()
    };
    let names: Vec<String> = c.y.iter().map(|s| one(&s.name)).collect();
    let mut ys: Vec<Vec<f64>> = vec![Vec::new(); c.y.len()];
    let rows: Option<(u32, u32, Vec<usize>)> = match c.source {
        Source::FoundSet => Some((table, to, found.to_vec())),
        Source::Related(rto) => {
            let rt = db.table_of(rto).map_or(0, |t| t.id);
            Some((
                rt,
                rto,
                record.map_or(Vec::new(), |r| db.related(to, r, rto)),
            ))
        }
        Source::CurrentRecord => None,
    };
    match rows {
        Some((t, cto, rows)) => {
            for r in rows {
                let ev = |k: &[u8]| db.eval_tokens(k, t, cto, Some(r));
                d.categories
                    .push(c.x.as_deref().map(|k| ev(k).to_text()).unwrap_or_default());
                for (i, s) in c.y.iter().enumerate() {
                    ys[i].push(number(&ev(&s.value)));
                }
                if let Some(k) = &c.size {
                    d.sizes.push(number(&ev(k)));
                }
            }
        }
        None => {
            let list = |k: &[u8]| lines(&db.eval_tokens(k, table, to, record).to_text());
            d.categories = c.x.as_deref().map(list).unwrap_or_default();
            for (i, s) in c.y.iter().enumerate() {
                ys[i] = list(&s.value)
                    .iter()
                    .map(|v| number(&calc::Value::text(v.as_str())))
                    .collect();
            }
            if let Some(k) = &c.size {
                d.sizes = list(k)
                    .iter()
                    .map(|v| number(&calc::Value::text(v.as_str())))
                    .collect();
            }
            let n = ys.iter().map(Vec::len).max().unwrap_or(0);
            d.categories
                .resize(n.max(d.categories.len()), String::new());
        }
    }
    d.series = names.into_iter().zip(ys).collect();
    d
}

/// What to paint, in points from the page (or screen) origin.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: Color,
        stroke: Option<Color>,
    },
    Line {
        a: (f64, f64),
        b: (f64, f64),
        width: f64,
        color: Color,
    },
    /// A filled polygon (areas, pie slices), star-shaped from its first
    /// point (it can be filled as a triangle fan from there).
    Polygon {
        points: Vec<(f64, f64)>,
        fill: Color,
    },
    Circle {
        c: (f64, f64),
        r: f64,
        fill: Color,
        stroke: Color,
    },
    /// Text whose left edge is at `x` and baseline at `y`.
    Text {
        x: f64,
        y: f64,
        size: f32,
        color: Color,
        text: String,
    },
}

fn rgb(c: u32) -> Color {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

/// The scheme's colours, sampled from FileMaker's Browse mode. Other
/// schemes fall back to Earth.
/// ponytail: only Earth (3) and Bright (0) are sampled; add the others
/// (Jewel Tones, Pastel, Gray…) when a file uses them.
pub fn palette(scheme: u32) -> Vec<Color> {
    let earth: [u32; 12] = [
        0x909A3F, 0xBD9D7D, 0x4E7E99, 0xCAA465, 0x8E8E8E, 0x7A7841, 0xDAC582, 0x4D7D98, 0x646567,
        0x60ABB3, 0x6F7349, 0x847566,
    ];
    let bright: [u32; 12] = [
        0x3E769B, 0xBBD0DF, 0xA5AC74, 0xD49147, 0x797BAA, 0xE4C881, 0xC35B25, 0x95B9C4, 0xC5B281,
        0xF7D775, 0x496374, 0x838C93,
    ];
    let p: &[u32] = if scheme == 0 { &bright } else { &earth };
    p.iter().map(|c| rgb(*c)).collect()
}

const TEXT: u32 = 0x333333;
const FRAME: u32 = 0xE6E6E6;

/// Darker edge of a mark, as FileMaker draws it.
fn edge(c: Color) -> Color {
    [c[0] * 0.75, c[1] * 0.75, c[2] * 0.75, 1.0]
}

/// A "nice" axis: (min, max, step) covering `lo..=hi` in about `n` steps.
pub fn axis(lo: f64, hi: f64, n: usize) -> (f64, f64, f64) {
    let (lo, hi) = (lo.min(0.0), hi.max(0.0));
    let span = if hi > lo { hi - lo } else { 1.0 };
    let raw = span / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

/// Number as a chart label: no trailing zeros.
pub fn label(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

struct Out(Vec<Shape>);

impl Out {
    fn text(&mut self, x: f64, base: f64, size: f32, s: &str, align: f64) {
        let w = text_width(s, size, false) as f64;
        self.0.push(Shape::Text {
            x: x - w * align,
            y: base,
            size,
            color: rgb(TEXT),
            text: s.to_string(),
        });
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: Color, stroke: Option<Color>) {
        self.0.push(Shape::Rect {
            x,
            y,
            w,
            h,
            fill,
            stroke,
        });
    }
    fn line(&mut self, a: (f64, f64), b: (f64, f64), width: f64, color: Color) {
        self.0.push(Shape::Line { a, b, width, color });
    }
}

/// The chart drawn in the rectangle (x, y, w, h).
pub fn shapes(c: &Chart, d: &Data, (x, y, w, h): (f64, f64, f64, f64)) -> Vec<Shape> {
    let mut o = Out(Vec::new());
    let colors = palette(c.scheme);
    let color = |i: usize| colors[i % colors.len()];
    let (title_size, label_size, axis_size, legend_size) = (19.0, 12.0, 15.0, 11.0);
    let mut top = y + 4.0;
    if !d.title.is_empty() {
        o.text(x + w / 2.0, top + 17.0, title_size, &d.title, 0.5);
        top += 26.0;
    }
    // Positive/negative charts show their first series only.
    let series: &[(String, Vec<f64>)] = if c.kind == Kind::PositiveNegative {
        &d.series[..d.series.len().min(1)]
    } else {
        &d.series
    };
    let per_category = series.len() == 1 && !matches!(c.kind, Kind::Line | Kind::Area);
    if c.legend && !series.is_empty() {
        let items: Vec<(&str, f64)> = series
            .iter()
            .map(|(n, _)| (n.as_str(), text_width(n, legend_size, false) as f64 + 16.0))
            .collect();
        let total: f64 = items.iter().map(|i| i.1 + 14.0).sum::<f64>() - 14.0;
        let mut lx = x + (w - total) / 2.0;
        for (i, (name, iw)) in items.iter().enumerate() {
            let col = color(i);
            o.rect(lx, top + 4.0, 8.0, 8.0, col, Some(edge(col)));
            o.text(lx + 12.0, top + 12.0, legend_size, name, 0.0);
            lx += iw + 14.0;
        }
        top += 20.0;
    }
    let bottom = y + h - 4.0 - if d.x_title.is_empty() { 0.0 } else { 18.0 };
    if !d.x_title.is_empty() {
        o.text(x + w / 2.0, y + h - 6.0, axis_size, &d.x_title, 0.5);
    }
    if c.kind == Kind::Pie {
        pie(&mut o, d, series, &colors, (x, top, w, bottom - top));
        return o.0;
    }
    let horizontal = c.kind.horizontal();
    let stacked = c.kind.stacked();
    let n = d.categories.len();
    // Value range.
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for i in 0..n {
        let (mut pos, mut neg) = (0.0, 0.0);
        for (_, v) in series {
            let v = v.get(i).copied().unwrap_or(0.0);
            if stacked {
                if v >= 0.0 { pos += v } else { neg += v }
            } else {
                hi = hi.max(v);
                lo = lo.min(v);
            }
        }
        hi = hi.max(pos);
        lo = lo.min(neg);
    }
    let numeric_x = matches!(c.kind, Kind::Scatter | Kind::Bubble);
    let xs: Vec<f64> = d
        .categories
        .iter()
        .map(|s| number(&calc::Value::text(s.as_str())))
        .collect();
    // Plot rectangle: value labels on both sides (top and bottom for bars).
    // ponytail: FileMaker turns the Y title by 90°; it is written flat here.
    let left_title = if d.y_title.is_empty() {
        0.0
    } else {
        text_width(&d.y_title, axis_size, false) as f64 + 6.0
    };
    let (px0, py0, px1, py1);
    let ticks = |len: f64| ((len / 40.0) as usize).clamp(2, 10);
    let (amin, amax, astep) = axis(lo, hi, ticks(if horizontal { w } else { bottom - top }));
    let vlabels: Vec<String> = {
        let mut v = Vec::new();
        let mut t = amin;
        while t <= amax + astep / 2.0 {
            v.push(label(t));
            t += astep;
        }
        v
    };
    let vw = vlabels
        .iter()
        .map(|s| text_width(s, label_size, false) as f64)
        .fold(0.0, f64::max);
    if horizontal {
        let cw = d
            .categories
            .iter()
            .map(|s| text_width(s, label_size, false) as f64)
            .fold(0.0, f64::max);
        px0 = x + left_title + cw + 6.0;
        px1 = x + w - vw / 2.0 - 2.0;
        py0 = top + 16.0;
        py1 = bottom - 16.0;
    } else {
        px0 = x + left_title + vw + 6.0;
        px1 = x + w - vw - 6.0;
        py0 = top + 6.0;
        py1 = bottom - 16.0;
    }
    if px1 <= px0 || py1 <= py0 {
        return o.0;
    }
    if !d.y_title.is_empty() {
        o.text(x + 2.0, (py0 + py1) / 2.0 + 5.0, axis_size, &d.y_title, 0.0);
    }
    if c.kind == Kind::PositiveNegative {
        let zy = py1 - (0.0 - amin) / (amax - amin) * (py1 - py0);
        o.rect(px0, py0, px1 - px0, zy - py0, rgb(0xCCCCFF), None);
        o.rect(px0, zy, px1 - px0, py1 - zy, rgb(0xFFCCCC), None);
    }
    o.rect(px0, py0, px1 - px0, py1 - py0, [0.0; 4], Some(rgb(FRAME)));
    // Value scale: maps a value to a coordinate along the value axis.
    let vpos = |v: f64| {
        let f = (v - amin) / (amax - amin);
        if horizontal {
            px0 + f * (px1 - px0)
        } else {
            py1 - f * (py1 - py0)
        }
    };
    for (i, s) in vlabels.iter().enumerate() {
        let p = vpos(amin + i as f64 * astep);
        if horizontal {
            o.text(p, py0 - 4.0, label_size, s, 0.5);
            o.text(p, py1 + 13.0, label_size, s, 0.5);
        } else {
            o.text(px0 - 4.0, p + 4.0, label_size, s, 1.0);
            o.text(px1 + 4.0, p + 4.0, label_size, s, 0.0);
        }
    }
    // Numeric X axis (scatter, bubble).
    let (xmin, xmax, xstep) = {
        let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if lo.is_finite() {
            axis(lo, hi, ticks(px1 - px0))
        } else {
            (0.0, 1.0, 1.0)
        }
    };
    let xpos = |v: f64| px0 + (v - xmin) / (xmax - xmin) * (px1 - px0);
    if numeric_x {
        let mut t = xmin;
        while t <= xmax + xstep / 2.0 {
            o.text(xpos(t), py1 + 13.0, label_size, &label(t), 0.5);
            t += xstep;
        }
    }
    if n == 0 {
        return o.0;
    }
    let band = if horizontal { py1 - py0 } else { px1 - px0 } / n as f64;
    let center = |i: usize| {
        if horizontal {
            py0 + band * (i as f64 + 0.5)
        } else {
            px0 + band * (i as f64 + 0.5)
        }
    };
    if !numeric_x {
        for (i, s) in d.categories.iter().enumerate() {
            if horizontal {
                o.text(px0 - 4.0, center(i) + 4.0, label_size, s, 1.0);
            } else {
                o.text(center(i), py1 + 13.0, label_size, s, 0.5);
            }
        }
    }
    let zero = vpos(0.0f64.clamp(amin, amax));
    match c.kind {
        Kind::Line | Kind::Area => {
            for (si, (_, v)) in series.iter().enumerate().rev() {
                let col = color(si);
                let pts: Vec<(f64, f64)> = v
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (center(i), vpos(*v)))
                    .collect();
                if c.kind == Kind::Area {
                    // One trapezoid per segment: convex, so every painter
                    // can fill it as a triangle fan.
                    for p in pts.windows(2) {
                        o.0.push(Shape::Polygon {
                            points: vec![p[0], p[1], (p[1].0, zero), (p[0].0, zero)],
                            fill: col,
                        });
                    }
                }
                for p in pts.windows(2) {
                    o.line(p[0], p[1], 1.5, edge(col));
                }
                if c.kind == Kind::Line {
                    for p in &pts {
                        o.0.push(Shape::Circle {
                            c: *p,
                            r: 4.0,
                            fill: col,
                            stroke: edge(col),
                        });
                    }
                }
                if c.data_points {
                    for (p, v) in pts.iter().zip(v) {
                        o.text(p.0, p.1 - 6.0, label_size, &label(*v), 0.5);
                    }
                }
            }
        }
        Kind::Scatter | Kind::Bubble => {
            let smax = d.sizes.iter().copied().fold(0.0, f64::max);
            for (si, (_, v)) in series.iter().enumerate() {
                let col = color(si);
                for (i, v) in v.iter().enumerate() {
                    let r = match (c.kind, d.sizes.get(i)) {
                        (Kind::Bubble, Some(s)) if smax > 0.0 => {
                            (s.abs() / smax).sqrt() * band.min(py1 - py0) / 3.0 + 2.0
                        }
                        _ => 4.0,
                    };
                    let p = (xpos(xs.get(i).copied().unwrap_or(0.0)), vpos(*v));
                    o.0.push(Shape::Circle {
                        c: p,
                        r,
                        fill: col,
                        stroke: edge(col),
                    });
                }
            }
        }
        _ => {
            // Columns and bars: side by side, or stacked.
            let groups = if stacked { 1 } else { series.len().max(1) };
            let gap = band * 0.25;
            let bw = (band - gap) / groups as f64;
            for i in 0..n {
                let (mut pos, mut neg) = (0.0, 0.0);
                for (si, (_, v)) in series.iter().enumerate() {
                    let val = v.get(i).copied().unwrap_or(0.0);
                    let col = if per_category { color(i) } else { color(si) };
                    let (from, to) = if stacked {
                        let base = if val >= 0.0 { &mut pos } else { &mut neg };
                        let f = *base;
                        *base += val;
                        (f, *base)
                    } else {
                        (0.0, val)
                    };
                    let slot = if stacked { 0 } else { si };
                    let a = center(i) - band / 2.0 + gap / 2.0 + bw * slot as f64;
                    let (v0, v1) = (vpos(from), vpos(to));
                    if horizontal {
                        o.rect(v0.min(v1), a, (v1 - v0).abs(), bw, col, Some(edge(col)));
                    } else {
                        o.rect(a, v0.min(v1), bw, (v1 - v0).abs(), col, Some(edge(col)));
                    }
                    if c.data_points {
                        let t = label(val);
                        if horizontal {
                            o.text(v1 + 3.0, a + bw / 2.0 + 4.0, label_size, &t, 0.0);
                        } else {
                            let ty = if val >= 0.0 { v1 - 3.0 } else { v1 + 12.0 };
                            o.text(a + bw / 2.0, ty, label_size, &t, 0.5);
                        }
                    }
                }
            }
        }
    }
    o.0
}

fn pie(
    o: &mut Out,
    d: &Data,
    series: &[(String, Vec<f64>)],
    colors: &[Color],
    r: (f64, f64, f64, f64),
) {
    let Some((_, v)) = series.first() else { return };
    let total: f64 = v.iter().map(|x| x.abs()).sum();
    if total <= 0.0 {
        return;
    }
    let rad = (r.2.min(r.3) / 2.0 - 4.0).max(1.0);
    let c = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
    // From 12 o'clock, clockwise.
    let mut a = -std::f64::consts::FRAC_PI_2;
    for (i, x) in v.iter().enumerate() {
        let sweep = x.abs() / total * std::f64::consts::TAU;
        let steps = ((sweep * 20.0) as usize).max(2);
        let mut pts = vec![c];
        for s in 0..=steps {
            let t = a + sweep * s as f64 / steps as f64;
            pts.push((c.0 + rad * t.cos(), c.1 + rad * t.sin()));
        }
        o.0.push(Shape::Polygon {
            points: pts,
            fill: colors[i % colors.len()],
        });
        let mid = a + sweep / 2.0;
        if let Some(name) = d.categories.get(i) {
            let lp = (
                c.0 + rad * 0.65 * mid.cos(),
                c.1 + rad * 0.65 * mid.sin() + 4.0,
            );
            o.text(lp.0, lp.1, 11.0, name, 0.5);
        }
        a += sweep;
    }
}

/// Paints shapes into the current PDF page.
pub fn paint_pdf(out: &mut crate::pdf::Pdf, shapes: &[Shape]) {
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
                if fill[3] > 0.0 {
                    out.fill_rect(*x, *y, *w, *h, *fill);
                }
                if let Some(c) = stroke {
                    let (x1, y1) = (x + w, y + h);
                    for (a, b) in [
                        ((*x, *y), (x1, *y)),
                        ((x1, *y), (x1, y1)),
                        ((x1, y1), (*x, y1)),
                        ((*x, y1), (*x, *y)),
                    ] {
                        out.line(a, b, 0.5, *c);
                    }
                }
            }
            Shape::Line { a, b, width, color } => out.line(*a, *b, *width, *color),
            Shape::Polygon { points, fill } => out.polygon(points, *fill),
            Shape::Circle { c, r, fill, .. } => {
                let pts: Vec<(f64, f64)> = (0..24)
                    .map(|i| {
                        let t = i as f64 / 24.0 * std::f64::consts::TAU;
                        (c.0 + r * t.cos(), c.1 + r * t.sin())
                    })
                    .collect();
                out.polygon(&pts, *fill);
            }
            Shape::Text {
                x,
                y,
                size,
                color,
                text,
            } => {
                let w = text_width(text, *size, false) as f64;
                out.text(
                    *x,
                    *y,
                    *size,
                    false,
                    *color,
                    text,
                    (*x - 1.0, y - 20.0, w + 2.0, 30.0),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_axes() {
        assert_eq!(axis(0.0, 180.0, 4), (0.0, 200.0, 50.0));
        assert_eq!(axis(-20.0, 80.0, 4), (-25.0, 100.0, 25.0));
        assert_eq!(label(2.50), "2.5");
    }

    #[test]
    fn columns_one_colour_per_category() {
        let c = Chart {
            scheme: 3,
            ..Chart::default()
        };
        let d = Data {
            categories: vec!["Jan".into(), "Feb".into()],
            series: vec![("S".into(), vec![120.0, 90.0])],
            ..Data::default()
        };
        let fills: Vec<Color> = shapes(&c, &d, (0.0, 0.0, 400.0, 200.0))
            .into_iter()
            .filter_map(|s| match s {
                Shape::Rect { fill, .. } if fill[3] > 0.0 => Some(fill),
                _ => None,
            })
            .collect();
        assert_eq!(fills, [palette(3)[0], palette(3)[1]]);
    }
}
