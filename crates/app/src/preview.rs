//! Preview mode: the found set laid out on pages, as FileMaker previews
//! and prints it (pagination from the `print` crate).

use eframe::egui::{self, Color32, Rect, Stroke, Vec2};
use fmp12::layouts::Layout;

use crate::look::{self, Look};
use crate::{Doc, merge_fields};

/// Draws page `page` of the preview and returns the page count.
pub fn draw(
    ui: &mut egui::Ui,
    doc: &Doc,
    layout: &Layout,
    found: &[usize],
    sort: &[(u32, engine::Order)],
    page: usize,
) -> usize {
    let paper = print::Paper::letter();
    let pages = print::paginate(&doc.db, layout, found, sort, &paper);
    let Some(p) = pages.get(page.min(pages.len().saturating_sub(1))) else {
        ui.label("—");
        return 0;
    };
    let table = doc
        .db
        .table_of((layout.table_occurrence & 0xFFF) as u32)
        .map_or(0, |t| t.id);
    let layout_to = (layout.table_occurrence & 0xFFF) as u32;
    let theme = doc.db.themes.iter().find(|x| x.number == layout.theme);
    let looks: Vec<Look> = layout
        .objects
        .iter()
        .map(|o| look::resolve(theme, &fmp12::theme::ObjectStyle::parse(&o.style_raw)))
        .collect();
    let (resp, painter) = ui.allocate_painter(
        Vec2::new(paper.width as f32 + 24.0, paper.height as f32 + 24.0),
        egui::Sense::hover(),
    );
    let page_rect = Rect::from_min_size(
        resp.rect.min + Vec2::splat(8.0),
        Vec2::new(paper.width as f32, paper.height as f32),
    );
    painter.rect_filled(
        page_rect.translate(Vec2::splat(3.0)),
        0.0,
        Color32::from_gray(170),
    );
    painter.rect_filled(page_rect, 0.0, Color32::WHITE);
    painter.rect_stroke(
        page_rect,
        0.0,
        Stroke::new(0.5, Color32::from_gray(140)),
        egui::StrokeKind::Inside,
    );
    let [mt, mr, mb, ml] = paper.margins;
    // Objects past the printable area are cut, as on paper.
    // ponytail: FileMaker prints them on extra pages across; not done yet.
    let painter = painter.with_clip_rect(Rect::from_min_max(
        page_rect.min + Vec2::new(ml as f32, mt as f32),
        page_rect.max - Vec2::new(mr as f32, mb as f32),
    ));
    // Front panel of each tab control: field 6, else its first panel.
    let front = |tc: u64| {
        let t = layout.objects.iter().find(|o| o.id == tc)?;
        t.active_child.or_else(|| {
            layout
                .objects
                .iter()
                .filter(|x| x.parent == Some(tc) && x.kind == 12)
                .map(|x| x.id)
                .min()
        })
    };
    let visible = |o: &fmp12::layouts::LayoutObject| {
        let mut p = o.parent;
        while let Some(id) = p {
            let Some(a) = layout.objects.iter().find(|x| x.id == id) else {
                break;
            };
            // ponytail: portal rows are not printed yet.
            if a.kind == 9 || (a.kind == 12 && a.parent.and_then(front) != Some(a.id)) {
                return false;
            }
            p = a.parent;
        }
        o.kind != 12
    };
    for band in &p.bands {
        let part = &layout.parts[band.part];
        let dy = mt + band.y - part.top;
        for (o, lk) in layout.objects.iter().zip(&looks).rev() {
            let b = layout.absolute(o);
            if b.top < part.top || b.top >= part.top + part.height || !visible(o) {
                continue;
            }
            let r = Rect::from_min_max(
                page_rect.min + Vec2::new((b.left + ml) as f32, (b.top + dy) as f32),
                page_rect.min + Vec2::new((b.right + ml) as f32, (b.bottom + dy) as f32),
            );
            lk.paint_box(&painter, r);
            let text = if let Some((to, field, _)) = o.field {
                let to = (to & 0xFFF) as u32;
                let rec = band.record;
                let tbl = doc.db.table_of(to).map(|x| x.id);
                let r = if to == layout_to || tbl == Some(table) {
                    rec
                } else {
                    rec.and_then(|r| doc.db.related(layout_to, r, to).first().copied())
                };
                let summary = tbl
                    .and_then(|t| doc.db.table(t)?.field(field as u32))
                    .is_some_and(|f| f.kind == fmp12::schema::FieldKind::Summary);
                match (tbl, r) {
                    _ if summary => print::field_text(
                        &doc.db,
                        layout,
                        found,
                        sort,
                        band,
                        o.field.unwrap_or_default(),
                    ),
                    (Some(tid), Some(r)) => doc.db.value(tid, r, field as u32).to_text(),
                    _ => String::new(),
                }
            } else if let Some(l) = o.label.as_ref().filter(|_| matches!(o.kind, 10 | 18)) {
                doc.db
                    .eval_tokens(l, table, layout_to, band.record)
                    .to_text()
            } else if let Some(t) = &o.text {
                merge_fields(doc, t, table, layout_to, band.record)
            } else {
                String::new()
            };
            if !text.is_empty() {
                lk.paint_text(&painter, r, &text.replace('\r', " "));
            }
        }
    }
    pages.len()
}
