//! Browse mode for the layout objects that need more than a box and a text:
//! popover buttons and their popovers, slide controls (swipe, dots), web
//! viewers (their address in a box, opened in the browser after a
//! confirmation; no browser engine), button bar segments, data formats,
//! conditional formats, "hide object when", and Tab / Shift+Tab following
//! the layout's tab order.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use fmp12::layouts::{Layout, LayoutObject, WEB_VIEWER, kind, slide};

use crate::Doc;
use crate::i18n::tr;
use crate::look::{self, Look};

/// What Browse mode remembers between frames.
#[derive(Default)]
pub struct State {
    /// The open popover.
    pub popover: Option<u64>,
    /// Screen rectangle of the open popover and of its button (clicks
    /// outside both close it).
    popover_rects: Vec<Rect>,
    /// A web viewer address waiting for "open in the browser?".
    pub open_url: Option<String>,
    /// Field object holding the keyboard focus.
    pub focus: Option<u64>,
}

/// Field object to focus this frame (Tab order), read by the field editor.
pub fn focus_id() -> egui::Id {
    egui::Id::NULL.with("basecraft_tab_focus")
}

/// Where an object's calculations are evaluated.
pub fn context(table: u32, to: u32, record: Option<usize>) -> print::objects::Context {
    print::objects::Context { table, to, record }
}

fn by_id(l: &Layout, id: u64) -> Option<&LayoutObject> {
    l.objects.iter().find(|o| o.id == id)
}

/// The popover holding `o` (itself if it is one).
pub fn popover_of(l: &Layout, o: &LayoutObject) -> Option<u64> {
    let mut cur = Some(o);
    while let Some(x) = cur {
        if x.kind == kind::POPOVER {
            return Some(x.id);
        }
        cur = x.parent.and_then(|p| by_id(l, p));
    }
    None
}

/// Front panel of a slide control: the user's choice, else field 6, else
/// its first panel.
pub fn front_slide(l: &Layout, sc: &LayoutObject, chosen: &BTreeMap<u64, u64>) -> Option<u64> {
    chosen
        .get(&sc.id)
        .copied()
        .or(sc.active_child)
        .or_else(|| panels(l, sc.id).first().copied())
}

fn panels(l: &Layout, sc: u64) -> Vec<u64> {
    let mut v: Vec<u64> = l
        .objects
        .iter()
        .filter(|x| x.parent == Some(sc) && x.kind == kind::SLIDE_PANEL)
        .map(|x| x.id)
        .collect();
    v.sort_unstable();
    v
}

/// Not drawn in Browse mode: inside a closed popover, on a slide panel
/// behind the front one, a panel itself, or hidden by its "hide object
/// when" calculation (or an enclosing object's).
pub fn skipped(
    doc: &Doc,
    l: &Layout,
    o: &LayoutObject,
    st: &State,
    chosen: &BTreeMap<u64, u64>,
    c: print::objects::Context,
) -> bool {
    if o.kind == kind::SLIDE_PANEL {
        return true;
    }
    if popover_of(l, o).is_some_and(|p| st.popover != Some(p)) {
        return true;
    }
    let mut cur = Some(o);
    while let Some(x) = cur {
        if print::objects::hidden(&doc.db, x, c) {
            return true;
        }
        if x.kind == kind::SLIDE_PANEL
            && let Some(sc) = x.parent.and_then(|p| by_id(l, p))
            && front_slide(l, sc, chosen) != Some(x.id)
        {
            return true;
        }
        cur = x.parent.and_then(|p| by_id(l, p));
    }
    false
}

/// The object's look with its conditional formats applied for `value`.
pub fn conditional_look(
    doc: &Doc,
    theme: Option<&fmp12::theme::Theme>,
    o: &LayoutObject,
    value: &calc::Value,
    c: print::objects::Context,
) -> Look {
    let style = print::objects::conditional_style(&doc.db, o, value, c);
    look::resolve(theme, &fmp12::theme::ObjectStyle::parse(&style))
}

/// A field's value as its data formats show it, with the negative colour.
pub fn formatted(
    doc: &Doc,
    o: &LayoutObject,
    v: &calc::Value,
) -> Option<(String, Option<Color32>)> {
    let (s, c) = print::format::apply(o.format.as_ref()?, v, &doc.db.locale)?;
    Some((s, c.map(|c| Color32::from_rgb(c[0], c[1], c[2]))))
}

/// The value an object is tested against by its conditional formats.
pub fn object_value(
    doc: &Doc,
    o: &LayoutObject,
    owner: Option<(u32, Option<usize>)>,
) -> calc::Value {
    match (o.field, owner) {
        (Some((_, f, rep)), Some((t, Some(r)))) => {
            doc.db.value_rep(t, r, f as u32, rep.max(1) as u32)
        }
        _ => calc::Value::text(o.text.clone().unwrap_or_default()),
    }
}

/// Order in which Browse mode paints objects: back to front, the open
/// popover and its objects last (on top).
pub fn paint_order(l: &Layout, st: &State) -> Vec<usize> {
    let mut v: Vec<usize> = (0..l.objects.len()).rev().collect();
    v.sort_by_key(|&i| popover_of(l, &l.objects[i]).is_some() && st.popover.is_some());
    v
}

fn label(doc: &Doc, o: &LayoutObject, c: print::objects::Context) -> String {
    match &o.label {
        Some(l) => doc.db.eval_tokens(l, c.table, c.to, c.record).to_text(),
        None => o.text.clone().unwrap_or_default(),
    }
}

/// A popover button: its label; a click opens (or closes) its popover.
#[allow(clippy::too_many_arguments)]
pub fn popover_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &Doc,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    st: &mut State,
    c: print::objects::Context,
) {
    lk.paint_box(painter, r);
    lk.paint_text(painter, r, &label(doc, o, c));
    let resp = ui
        .interact(
            r,
            ui.scope_id().with(("popover_button", o.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        st.popover = if st.popover == o.popover {
            None
        } else {
            o.popover
        };
        st.popover_rects.clear();
    }
    if st.popover.is_some() && st.popover == o.popover {
        st.popover_rects.push(r);
    }
}

/// An open popover: its box, and its title bar when it has a title.
#[allow(clippy::too_many_arguments)]
pub fn popover(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &Doc,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    st: &mut State,
    c: print::objects::Context,
) {
    painter.rect_filled(
        r.translate(Vec2::splat(3.0)),
        4.0,
        Color32::from_black_alpha(40),
    );
    if lk.background.is_none() {
        painter.rect_filled(r, 4.0, Color32::WHITE);
    }
    lk.paint_box(painter, r);
    painter.rect_stroke(
        r,
        4.0,
        Stroke::new(1.0, Color32::from_gray(170)),
        egui::StrokeKind::Inside,
    );
    let title = label(doc, o, c);
    if !title.is_empty() {
        let bar = Rect::from_min_size(r.min, Vec2::new(r.width(), 26.0));
        painter.rect_filled(bar, 4.0, Color32::from_gray(238));
        painter.text(
            bar.center(),
            egui::Align2::CENTER_CENTER,
            title,
            lk.font(),
            lk.color,
        );
    }
    // Clicks inside the popover do not reach what is under it.
    let _ = ui.interact(r, ui.scope_id().with(("popover", o.id)), Sense::click());
    st.popover_rects.push(r);
}

/// After painting: a click outside the open popover and its button closes it.
pub fn close_popover_on_click(ui: &egui::Ui, st: &mut State) {
    let Some(pos) = ui.input(|i| {
        i.pointer
            .any_click()
            .then(|| i.pointer.interact_pos())
            .flatten()
    }) else {
        st.popover_rects.clear();
        return;
    };
    if st.popover.is_some() && !st.popover_rects.iter().any(|r| r.contains(pos)) {
        st.popover = None;
    }
    st.popover_rects.clear();
}

/// A slide control: its box, the navigation dots (unless hidden) and
/// swiping between panels (unless off). The front panel's objects are
/// drawn by the caller.
#[allow(clippy::too_many_arguments)]
pub fn slide_control(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    l: &Layout,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    chosen: &mut BTreeMap<u64, u64>,
) {
    lk.paint_box(painter, r);
    let ps = panels(l, o.id);
    let front = front_slide(l, o, chosen);
    let at = ps.iter().position(|p| Some(*p) == front).unwrap_or(0);
    if o.flags & slide::NO_SWIPE == 0 {
        let resp = ui.interact(r, ui.scope_id().with(("slide", o.id)), Sense::drag());
        if resp.drag_stopped() {
            let dx = resp.drag_delta().x + ui.input(|i| i.pointer.delta().x);
            let total = ui.input(|i| {
                i.pointer
                    .press_origin()
                    .zip(i.pointer.interact_pos())
                    .map_or(dx, |(a, b)| b.x - a.x)
            });
            let next = if total < -30.0 {
                (at + 1).min(ps.len().saturating_sub(1))
            } else if total > 30.0 {
                at.saturating_sub(1)
            } else {
                at
            };
            if let Some(p) = ps.get(next) {
                chosen.insert(o.id, *p);
            }
        }
    }
    if o.flags & slide::NO_DOTS == 0 && ps.len() > 1 {
        let gap = 14.0;
        let x0 = r.center().x - gap * (ps.len() as f32 - 1.0) / 2.0;
        for (i, p) in ps.iter().enumerate() {
            let c = egui::pos2(x0 + gap * i as f32, r.bottom() - 10.0);
            let on = i == at;
            painter.circle_filled(c, 4.0, Color32::from_gray(if on { 60 } else { 190 }));
            let hit = Rect::from_center_size(c, Vec2::splat(12.0));
            if ui
                .interact(hit, ui.scope_id().with(("dot", o.id, *p)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                chosen.insert(o.id, *p);
            }
        }
    }
}

/// A web viewer: a placeholder box with its address and an "open in the
/// browser" button.
#[allow(clippy::too_many_arguments)]
pub fn web_viewer(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    doc: &Doc,
    o: &LayoutObject,
    lk: &Look,
    r: Rect,
    st: &mut State,
    c: print::objects::Context,
) -> bool {
    if o.external.is_none_or(|e| e.0 != WEB_VIEWER) {
        return false;
    }
    painter.rect_filled(r, 0.0, Color32::from_gray(246));
    painter.rect_stroke(
        r,
        0.0,
        Stroke::new(1.0, Color32::from_gray(190)),
        egui::StrokeKind::Inside,
    );
    let url = o
        .url
        .as_ref()
        .map(|u| doc.db.eval_tokens(u, c.table, c.to, c.record).to_text())
        .unwrap_or_default();
    painter.text(
        r.center() - Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        tr("Web Viewer"),
        egui::FontId::proportional(12.0),
        Color32::from_gray(120),
    );
    painter.with_clip_rect(r.shrink(4.0)).text(
        r.center() + Vec2::new(0.0, 8.0),
        egui::Align2::CENTER_CENTER,
        &url,
        lk.font(),
        Color32::from_rgb(30, 90, 170),
    );
    if !url.is_empty() {
        let b = Rect::from_min_size(
            r.right_top() + Vec2::new(-112.0, 4.0),
            Vec2::new(108.0, 20.0),
        );
        if ui
            .put(b, egui::Button::new(tr("Open in Browser…")).small())
            .clicked()
        {
            st.open_url = Some(url);
        }
    }
    true
}

/// The "open this address in the browser?" confirmation.
pub fn url_confirmation(ctx: &egui::Context, st: &mut State) {
    let Some(url) = st.open_url.clone() else {
        return;
    };
    let mut close = false;
    egui::Modal::new(egui::Id::NULL.with("open_url")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(tr("Open this address in your web browser?"));
        ui.monospace(&url);
        ui.horizontal(|ui| {
            if ui.button(tr("Open")).clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(&url));
                close = true;
            }
            if ui.button(tr("Cancel")).clicked() {
                close = true;
            }
        });
    });
    if close {
        st.open_url = None;
    }
}

/// Highlights the active segment of a button bar.
pub fn segment_highlight(painter: &egui::Painter, l: &Layout, o: &LayoutObject, r: Rect) {
    let active = o
        .parent
        .and_then(|p| by_id(l, p))
        .filter(|b| b.kind == kind::BUTTON_BAR)
        .and_then(|b| b.selected);
    if active == Some(o.id) {
        painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(40, 110, 200, 60));
    }
}

/// Tab / Shift+Tab: the next (previous) field of the tab order after the
/// focused one, to focus this frame.
pub fn tab_navigation(ui: &egui::Ui, l: &Layout, st: &State) {
    let (tab, back) = ui.input(|i| (i.key_pressed(egui::Key::Tab), i.modifiers.shift));
    let Some(cur) = st.focus.filter(|_| tab) else {
        return;
    };
    let order: Vec<u64> = l
        .tab_order
        .iter()
        .copied()
        .filter(|id| by_id(l, *id).is_some_and(|o| o.kind == kind::FIELD))
        .collect();
    let Some(i) = order.iter().position(|x| *x == cur) else {
        return;
    };
    let n = order.len();
    let next = if back {
        order[(i + n - 1) % n]
    } else {
        order[(i + 1) % n]
    };
    ui.ctx().data_mut(|d| d.insert_temp(focus_id(), next));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popover_objects_paint_last() {
        let o = |id, kind, parent| LayoutObject {
            id,
            kind,
            parent,
            ..Default::default()
        };
        let l = Layout {
            objects: vec![
                o(1, kind::POPOVER, None),
                o(2, kind::FIELD, Some(1)),
                o(3, kind::FIELD, None),
            ],
            ..Default::default()
        };
        let mut st = State::default();
        assert_eq!(paint_order(&l, &st), [2, 1, 0]);
        st.popover = Some(1);
        assert_eq!(paint_order(&l, &st), [2, 1, 0]);
        assert_eq!(popover_of(&l, &l.objects[1]), Some(1));
        assert_eq!(popover_of(&l, &l.objects[2]), None);
    }
}
