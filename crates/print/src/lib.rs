//! Preview mode and printing: lays a layout's parts out on pages the way
//! FileMaker's Preview mode does, and renders the pages to PDF.
//!
//! * the header (title header on page 1) and footer (title footer on
//!   page 1) repeat on every page;
//! * the body repeats once per record of the found set;
//! * leading and trailing sub-summaries print before and after each group
//!   of records when the found set is sorted by their break field;
//! * leading and trailing grand summaries print once, before and after
//!   the records;
//! * a part that does not fit in the space left on a page moves to the next.

pub mod chart;
pub mod look;
pub mod merge;
pub mod pdf;

use std::ops::Range;

use engine::{Database, Order};
use fmp12::layouts::{Layout, LayoutObject, part};
use fmp12::schema::FieldKind;
use fmp12::theme::ObjectStyle;

use look::{Look, resolve};
use pdf::{Pdf, text_width};

/// Paper size and margins, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    pub width: f64,
    pub height: f64,
    /// Top, right, bottom, left.
    pub margins: [f64; 4],
}

impl Paper {
    /// US Letter, portrait, half-inch margins.
    pub fn letter() -> Paper {
        Paper {
            width: 612.0,
            height: 792.0,
            margins: [36.0; 4],
        }
    }
    /// A4, portrait, half-inch margins.
    pub fn a4() -> Paper {
        Paper {
            width: 595.28,
            height: 841.89,
            margins: [36.0; 4],
        }
    }
    pub fn landscape(self) -> Paper {
        Paper {
            width: self.height,
            height: self.width,
            ..self
        }
    }
    pub fn with_margins(self, m: f64) -> Paper {
        Paper {
            margins: [m; 4],
            ..self
        }
    }
}

/// One part placed on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    /// Index into the layout's parts.
    pub part: usize,
    /// Top of the part, from the top margin.
    pub y: f64,
    /// Record whose fields the part shows (index into the table).
    pub record: Option<usize>,
    /// Records summary fields summarize, as a range of the found set.
    pub group: Range<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub bands: Vec<Band>,
}

fn layout_to(l: &Layout) -> u32 {
    (l.table_occurrence & 0xFFF) as u32
}

/// Lays `found` (in sort order `sort`) out on pages of `paper`.
pub fn paginate(
    db: &Database,
    layout: &Layout,
    found: &[usize],
    sort: &[(u32, Order)],
    paper: &Paper,
) -> Vec<Page> {
    let table = db.table_of(layout_to(layout)).map_or(0, |t| t.id);
    let kind = |k: u64| layout.parts.iter().position(|p| p.kind == k);
    let height = |i: Option<usize>| i.map_or(0.0, |i| layout.parts[i].height);
    let all = 0..found.len();
    let sort_fields: Vec<u32> = sort.iter().map(|(f, _)| *f).collect();
    // Sub-summaries whose break field is a sort field: (part, level, groups).
    let mut subs: Vec<(usize, usize, Vec<Range<usize>>)> = layout
        .parts
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            matches!(
                p.kind,
                part::LEADING_SUB_SUMMARY | part::TRAILING_SUB_SUMMARY
            )
        })
        .filter_map(|(i, p)| {
            let (_, f) = p.break_field?;
            let level = sort_fields.iter().position(|s| *s == f)?;
            Some((
                i,
                level,
                engine::groups(db, table, found, &sort_fields[..=level]),
            ))
        })
        .collect();
    subs.sort_by_key(|(_, level, _)| *level);
    // The flow of parts between header and footer.
    let mut flow: Vec<(usize, Option<usize>, Range<usize>)> = Vec::new();
    let first = |r: &Range<usize>| found.get(r.start).copied();
    if let Some(p) = kind(part::LEADING_GRAND_SUMMARY) {
        flow.push((p, first(&all), all.clone()));
    }
    let body = kind(part::BODY);
    for (i, &rec) in found.iter().enumerate() {
        for (p, _, groups) in &subs {
            if layout.parts[*p].kind != part::LEADING_SUB_SUMMARY {
                continue;
            }
            if let Some(g) = groups.iter().find(|g| g.start == i) {
                flow.push((*p, first(g), g.clone()));
            }
        }
        if let Some(b) = body {
            flow.push((b, Some(rec), all.clone()));
        }
        for (p, _, groups) in subs.iter().rev() {
            if layout.parts[*p].kind != part::TRAILING_SUB_SUMMARY {
                continue;
            }
            if let Some(g) = groups.iter().find(|g| g.end == i + 1) {
                flow.push((*p, first(g), g.clone()));
            }
        }
    }
    if let Some(p) = kind(part::TRAILING_GRAND_SUMMARY) {
        flow.push((p, first(&all), all.clone()));
    }

    let avail = paper.height - paper.margins[0] - paper.margins[2];
    let header = |page: usize| {
        (page == 0)
            .then(|| kind(part::TITLE_HEADER))
            .flatten()
            .or(kind(part::HEADER))
    };
    let footer = |page: usize| {
        (page == 0)
            .then(|| kind(part::TITLE_FOOTER))
            .flatten()
            .or(kind(part::FOOTER))
    };
    let mut pages: Vec<Page> = Vec::new();
    let mut flow = flow.into_iter().peekable();
    loop {
        let n = pages.len();
        let mut page = Page::default();
        let mut y = height(header(n));
        let bottom = avail - height(footer(n));
        while let Some((p, record, group)) = flow.peek().cloned() {
            let h = layout.parts[p].height;
            // A part taller than a page still goes on its own page.
            if y + h > bottom && !page.bands.is_empty() {
                break;
            }
            page.bands.push(Band {
                part: p,
                y,
                record,
                group,
            });
            y += h;
            flow.next();
        }
        // Header shows the page's first record, footer its last.
        let recs: Vec<usize> = page
            .bands
            .iter()
            .filter(|b| Some(b.part) == body || body.is_none())
            .filter_map(|b| b.record)
            .collect();
        if let Some(h) = header(n) {
            page.bands.insert(
                0,
                Band {
                    part: h,
                    y: 0.0,
                    record: recs.first().copied(),
                    group: all.clone(),
                },
            );
        }
        if let Some(f) = footer(n) {
            page.bands.push(Band {
                part: f,
                y: avail - height(Some(f)),
                record: recs.last().copied(),
                group: all.clone(),
            });
        }
        pages.push(page);
        if flow.peek().is_none() {
            break;
        }
    }
    pages
}

/// Text field object `field` (occurrence, field, repetition) shows on
/// `band` of the pages of `found` sorted by `sort` (summary fields over
/// the band's records).
pub fn field_text(
    db: &Database,
    layout: &Layout,
    found: &[usize],
    sort: &[(u32, Order)],
    band: &Band,
    field: (u64, u64, u64),
) -> String {
    let sort: Vec<u32> = sort.iter().map(|(f, _)| *f).collect();
    let ctx = Ctx {
        db,
        layout,
        theme: None,
        found,
        sort: &sort,
        page: 0,
        pages: 0,
    };
    let group = found.get(band.group.clone()).unwrap_or_default();
    let body = layout
        .parts
        .get(band.part)
        .is_some_and(|p| p.kind == part::BODY);
    ctx.field_text(field, band.record, group, body)
}

/// What a band draws its objects with.
struct Ctx<'a> {
    db: &'a Database,
    layout: &'a Layout,
    theme: Option<&'a fmp12::theme::Theme>,
    found: &'a [usize],
    /// Sort fields of the found set, in order.
    sort: &'a [u32],
    page: usize,
    pages: usize,
}

impl Ctx<'_> {
    fn find(&self, id: u64) -> Option<&LayoutObject> {
        self.layout.objects.iter().find(|o| o.id == id)
    }

    /// Front panel of a tab control: field 6, else its first panel.
    fn front(&self, tc: &LayoutObject) -> Option<u64> {
        tc.active_child.or_else(|| {
            self.layout
                .objects
                .iter()
                .filter(|x| x.parent == Some(tc.id) && x.kind == 12)
                .map(|x| x.id)
                .min()
        })
    }

    /// Hidden behind another tab, a panel itself, or drawn by its portal.
    fn skipped(&self, o: &LayoutObject) -> bool {
        if o.kind == 12 {
            return true;
        }
        let mut p = o.parent;
        while let Some(id) = p {
            let Some(a) = self.find(id) else { break };
            if a.kind == 9 {
                return true;
            }
            if a.kind == 12 {
                let tc = a.parent.and_then(|t| self.find(t));
                if tc.is_some_and(|tc| self.front(tc) != Some(a.id)) {
                    return true;
                }
            }
            p = a.parent;
        }
        false
    }

    fn text(&self, v: &calc::Value) -> String {
        engine::stored_text(v, &self.db.locale)
    }

    /// Text a field object shows for `record`, summary fields over `group`.
    fn field_text(
        &self,
        field: (u64, u64, u64),
        record: Option<usize>,
        group: &[usize],
        body: bool,
    ) -> String {
        let (to, fid) = ((field.0 & 0xFFF) as u32, field.1 as u32);
        let Some(t) = self.db.table_of(to) else {
            return String::new();
        };
        let Some(f) = t.field(fid) else {
            return String::new();
        };
        let from = layout_to(self.layout);
        if f.kind == FieldKind::Summary {
            let v = match record {
                // Running summaries and fractions show the record's own value.
                Some(r) if to == from && body => {
                    engine::summary_at(self.db, t.id, fid, r, self.found, self.sort)
                }
                _ if to == from => {
                    engine::summarize_sorted(self.db, t.id, fid, group, self.found, self.sort)
                }
                _ => {
                    let set = record.map_or_else(Vec::new, |r| self.db.related(from, r, to));
                    engine::summarize(self.db, t.id, fid, &set, &set)
                }
            };
            return self.text(&v);
        }
        let Some(r) = record else {
            return String::new();
        };
        engine::export::values_through(self.db, from, r, to, fid, self.found)
            .first()
            .map(|v| self.text(v))
            .unwrap_or_default()
    }

    /// `<<Field>>`, `<<TO::Field>>`, `{{PageNumber}}` and `{{PageCount}}` in layout text.
    fn merge(&self, text: &str, record: Option<usize>, group: &[usize], body: bool) -> String {
        let text = text
            .replace("{{PageNumber}}", &self.page.to_string())
            .replace("{{PageCount}}", &self.pages.to_string());
        let mut out = String::new();
        let mut rest = text.as_str();
        while let Some(a) = rest.find("<<") {
            out.push_str(&rest[..a]);
            let Some(b) = rest[a..].find(">>") else {
                break;
            };
            let name = &rest[a + 2..a + b];
            let (to_name, field) = name.split_once("::").unwrap_or(("", name));
            let to = self
                .db
                .occurrences
                .iter()
                .find(|o| o.name.eq_ignore_ascii_case(to_name))
                .map_or(layout_to(self.layout), |o| o.id);
            if let Some(f) = self.db.table_of(to).and_then(|t| t.field_by_name(field)) {
                out.push_str(&self.field_text((to as u64, f.id as u64, 1), record, group, body));
            }
            rest = &rest[a + b + 2..];
        }
        out.push_str(rest);
        out
    }
}

/// (x, y, width, height) on the page.
type Rect = (f64, f64, f64, f64);

fn paint_box(pdf: &mut Pdf, lk: &Look, r: Rect) {
    let (x, y, w, h) = r;
    if let Some(bg) = lk.background {
        pdf.fill_rect(x, y, w, h, bg);
    }
    let [t, ri, b, le] = lk.borders;
    let side = |pdf: &mut Pdf, s: (f32, look::Color), a: (f64, f64), z: (f64, f64)| {
        if s.0 > 0.0 && s.1[3] > 0.0 {
            pdf.line(a, z, s.0 as f64, s.1);
        }
    };
    side(pdf, t, (x, y), (x + w, y));
    side(pdf, ri, (x + w, y), (x + w, y + h));
    side(pdf, b, (x, y + h), (x + w, y + h));
    side(pdf, le, (x, y), (x, y + h));
}

fn paint_text(pdf: &mut Pdf, lk: &Look, r: Rect, text: &str) {
    if text.is_empty() {
        return;
    }
    let (x, y, w, h) = r;
    let [pt, pr, pb, pl] = lk.padding.map(|p| p as f64);
    let (cx, cy, cw, ch) = (
        x + pl,
        y + pt,
        (w - pl - pr).max(0.0),
        (h - pt - pb).max(0.0),
    );
    let size = lk.font_size.max(4.0);
    let line_h = size as f64 * 1.2;
    let lines: Vec<&str> = text.split(['\r', '\n']).collect();
    let block = line_h * lines.len() as f64;
    let top = match lk.valign {
        0 => cy,
        2 => cy + ch - block,
        _ => cy + (ch - block) / 2.0,
    };
    for (i, line) in lines.iter().enumerate() {
        let lw = text_width(line, size, lk.bold) as f64;
        let lx = match lk.align {
            1 => cx + (cw - lw) / 2.0,
            2 => cx + cw - lw,
            _ => cx,
        };
        // Baseline at about 80 % of the line box.
        let base = top + line_h * i as f64 + size as f64 * 0.95;
        pdf.text(lx, base, size, lk.bold, lk.color, line, (x, y, w, h));
    }
}

/// Renders found set `found` (sorted by `sort`) on layout `layout_id` to PDF.
/// Returns the PDF bytes and the page count.
pub fn pdf(
    db: &Database,
    layout_id: u32,
    found: &[usize],
    sort: &[(u32, Order)],
    paper: &Paper,
) -> Result<(Vec<u8>, usize), String> {
    pdf_with_charts(db, &Charts::new(), layout_id, found, sort, paper)
}

/// Charts by (layout id, object id), see [`fmp12::charts::charts`].
pub type Charts = std::collections::BTreeMap<(u32, u64), fmp12::charts::Chart>;

/// [`pdf`], drawing the layout's charts too.
pub fn pdf_with_charts(
    db: &Database,
    charts: &Charts,
    layout_id: u32,
    found: &[usize],
    sort: &[(u32, Order)],
    paper: &Paper,
) -> Result<(Vec<u8>, usize), String> {
    let layout = db
        .layouts
        .iter()
        .find(|l| l.id == layout_id)
        .ok_or_else(|| format!("no layout {layout_id}"))?;
    let pages = paginate(db, layout, found, sort, paper);
    let sort_fields: Vec<u32> = sort.iter().map(|(f, _)| *f).collect();
    let mut ctx = Ctx {
        db,
        layout,
        theme: db.themes.iter().find(|t| t.number == layout.theme),
        found,
        sort: &sort_fields,
        page: 0,
        pages: pages.len(),
    };
    let looks: Vec<Look> = layout
        .objects
        .iter()
        .map(|o| resolve(ctx.theme, &ObjectStyle::parse(&o.style_raw)))
        .collect();
    let mut out = Pdf::new();
    let [mt, _, _, ml] = paper.margins;
    for (n, page) in pages.iter().enumerate() {
        ctx.page = n + 1;
        out.page(paper.width, paper.height);
        for band in &page.bands {
            let p = &layout.parts[band.part];
            let group: Vec<usize> = found[band.group.clone()].to_vec();
            let body = p.kind == part::BODY;
            let dy = mt + band.y - p.top;
            let plook = resolve(ctx.theme, &ObjectStyle::parse(&p.style_raw));
            if let Some(bg) = plook.background {
                out.fill_rect(ml, mt + band.y, layout.width.max(1.0), p.height, bg);
            }
            // Objects are stored front to back: paint them in reverse.
            for (o, lk) in layout.objects.iter().zip(&looks).rev() {
                let ab = layout.absolute(o);
                if ab.top < p.top || ab.top >= p.top + p.height || ctx.skipped(o) {
                    continue;
                }
                let r = (
                    ml + ab.left,
                    ab.top + dy,
                    ab.right - ab.left,
                    ab.bottom - ab.top,
                );
                if let Some(c) = charts.get(&(layout.id, o.id)) {
                    let d = chart::data(db, c, layout_to(layout), found, band.record);
                    chart::paint_pdf(&mut out, &chart::shapes(c, &d, r));
                    continue;
                }
                draw(&mut out, &ctx, o, lk, r, band.record, &group, body);
                if o.kind == 9 {
                    portal(&mut out, &ctx, o, &looks, r, band.record);
                }
            }
        }
    }
    Ok((out.finish(), pages.len()))
}

#[allow(clippy::too_many_arguments)]
fn draw(
    out: &mut Pdf,
    ctx: &Ctx,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    record: Option<usize>,
    group: &[usize],
    body: bool,
) {
    paint_box(out, lk, r);
    let text = if let Some(f) = o.field {
        ctx.field_text(f, record, group, body)
    } else if let Some(l) = o.label.as_ref().filter(|_| matches!(o.kind, 10 | 18)) {
        let t = ctx.db.table_of(layout_to(ctx.layout)).map_or(0, |t| t.id);
        ctx.db
            .eval_tokens(l, t, layout_to(ctx.layout), record)
            .to_text()
    } else if let Some(t) = &o.text {
        ctx.merge(t, record, group, body)
    } else {
        return;
    };
    paint_text(out, lk, r, &text);
}

/// Portal rows: the related records of `record`, one row each.
fn portal(
    out: &mut Pdf,
    ctx: &Ctx,
    o: &LayoutObject,
    looks: &[Look],
    r: Rect,
    record: Option<usize>,
) {
    let (Some((pto, rows, first)), Some(rec)) = (o.portal, record) else {
        return;
    };
    let pto = (pto & 0xFFF) as u32;
    let related = ctx.db.related(layout_to(ctx.layout), rec, pto);
    let rows = rows.max(1) as usize;
    let row_h = r.3 / rows as f64;
    for row in 0..rows {
        let Some(&rr) = related.get(first.saturating_sub(1) as usize + row) else {
            break;
        };
        for (c, lk) in ctx.layout.objects.iter().zip(looks).rev() {
            if c.parent != Some(o.id) {
                continue;
            }
            let cr = (
                r.0 + c.bounds.left,
                r.1 + row as f64 * row_h + c.bounds.top,
                c.bounds.right - c.bounds.left,
                c.bounds.bottom - c.bounds.top,
            );
            paint_box(out, lk, cr);
            if let Some((to, fid, _)) = c.field {
                let to = (to & 0xFFF) as u32;
                if let Some(t) = ctx.db.table_of(to) {
                    let v = ctx.db.value(t.id, rr, fid as u32);
                    paint_text(out, lk, cr, &ctx.text(&v));
                }
            } else if let Some(t) = &c.text {
                paint_text(out, lk, cr, t);
            }
        }
    }
}

#[cfg(test)]
mod tests;
