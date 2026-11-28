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
    // Get ( PageNumber ) for this page; PageCount only while printing.
    calc::functions::set_page(page.min(pages.len().saturating_sub(1)) + 1, None);
    let sort_fields: Vec<u32> = sort.iter().map(|(f, _)| *f).collect();
    let found_set = engine::FoundSet {
        table,
        records: found,
        sort: &sort_fields,
    };
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
                .filter(|x| x.parent == Some(tc) && matches!(x.kind, 12 | 17))
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
            // Popovers do not print; tab and slide panels only the front one.
            if matches!(a.kind, 9 | 15)
                || (matches!(a.kind, 12 | 17) && a.parent.and_then(front) != Some(a.id))
            {
                return false;
            }
            p = a.parent;
        }
        !matches!(o.kind, 12 | 15 | 17)
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
            if crate::chart::draw(ui.ctx(), &painter, doc, layout, o, r, found, band.record) {
                continue;
            }
            let oc = crate::objects::context(table, layout_to, band.record);
            if print::objects::hidden(&doc.db, o, oc) {
                continue;
            }
            let value = o.field.and_then(|(to, field, _)| {
                let to = (to & 0xFFF) as u32;
                let tbl = doc.db.table_of(to)?.id;
                // Summaries are computed over the band's records below.
                let kind = doc.db.table(tbl)?.field(field as u32)?.kind;
                if kind == fmp12::schema::FieldKind::Summary {
                    return None;
                }
                let rec = if to == layout_to || tbl == table {
                    band.record
                } else {
                    band.record
                        .and_then(|r| doc.db.related(layout_to, r, to).first().copied())
                }?;
                Some(doc.db.value(tbl, rec, field as u32))
            });
            let cond_look;
            let lk = if o.conditions.is_empty() {
                lk
            } else {
                let v = value
                    .clone()
                    .unwrap_or_else(|| calc::Value::text(o.text.clone().unwrap_or_default()));
                cond_look = crate::objects::conditional_look(doc, theme, o, &v, oc);
                &cond_look
            };
            lk.paint_box(&painter, r);
            if let Some(p) = o.picture.and_then(|id| doc.db.pictures.get(&id)) {
                crate::container::picture(ui.ctx(), &painter, r, p.data());
            }
            if let Some(u) = o.url.as_ref() {
                let url = doc
                    .db
                    .eval_tokens(u, table, layout_to, band.record)
                    .to_text();
                lk.paint_text(&painter, r, &url);
                continue;
            }
            if let Some((s, color)) = value
                .as_ref()
                .and_then(|v| crate::objects::formatted(doc, o, v))
            {
                let mut l2 = lk.clone();
                l2.color = color.unwrap_or(lk.color);
                l2.paint_text(&painter, r, &s);
                continue;
            }
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
            } else if let Some(l) = o.label.as_ref().filter(|_| matches!(o.kind, 10 | 14 | 18)) {
                doc.db
                    .eval_tokens(l, table, layout_to, band.record)
                    .to_text()
            } else if o.text.is_some() {
                lk.paint_styled(&painter, r, o, |s| {
                    let s = print::symbols(&doc.db, s, layout_to, band.record, Some(found_set));
                    merge_fields(doc, &Default::default(), &s, table, layout_to, band.record)
                });
                continue;
            } else {
                String::new()
            };
            if !text.is_empty() {
                // FileMaker's line breaks (`\r`) break lines, as in Browse.
                lk.paint_text(&painter, r, &text);
            }
        }
    }
    calc::functions::set_page(0, None);
    pages.len()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use eframe::egui;

    /// Preview shows `{{PageNumber}}` (and `Get ( PageNumber )`) for the
    /// page drawn; PageCount only while printing; both reset after.
    #[test]
    fn page_number_symbol_in_preview() {
        let app = crate::ui_tests::shop_app();
        let d = app.doc.as_ref().unwrap();
        let mut l = d.layouts[app.layout].clone();
        let top = l.parts.iter().find(|p| p.kind == 4).unwrap().top + 40.0;
        l.objects.push(fmp12::layouts::LayoutObject {
            id: 900,
            kind: 2,
            bounds: fmp12::layouts::Bounds {
                top,
                left: 10.0,
                bottom: top + 20.0,
                right: 200.0,
            },
            text: Some("Pg {{PageNumber}}/{{PageCount}}.".into()),
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::draw(ui, d, &l, &[0], &[], 0);
        });
        let texts: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        out.textures_delta.clear();
        assert!(texts.iter().any(|t| t == "Pg 1/."), "{texts:?}");
        let flag = d.db.eval_source("Get ( PageNumber )", 0, 0, None);
        assert_eq!(flag.to_text(), "0");
    }
}
