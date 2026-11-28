//! Browse/Find mode widgets of the field control styles that are not an
//! edit box or a list: checkbox sets, radio button sets and the drop-down
//! calendar (FORMAT.md §10, field 8.5).

use eframe::egui::{self, Rect, RichText, Vec2};

use crate::look::Look;

/// A concealed edit box: an edit box that shows bullets, not its text.
pub(crate) fn concealed(o: &fmp12::layouts::LayoutObject) -> bool {
    o.display == 0 && o.field_flags & fmp12::layouts::field_flags::CONCEALED != 0
}

/// A checkbox set (`display` 3) or radio button set (4) of `values`, laid
/// out in columns filling `r`: as many rows as fit in its height, then the
/// next column. Returns the field's new text when the user clicks one.
// ponytail: column-major order and an even column width are not checked
// against FileMaker; "Other…" (`field_flags::OTHER`) and "Allow editing"
// (`field_flags::EDIT_VALUES`, pop-ups and drop-downs only) are not drawn.
pub(crate) fn value_set(
    ui: &mut egui::Ui,
    lk: &Look,
    r: Rect,
    display: u64,
    values: &[String],
    current: &str,
) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let line = lk.font().size + 6.0;
    let rows = ((r.height() / line).floor() as usize).max(1);
    let cols = values.len().div_ceil(rows);
    let w = r.width() / cols as f32;
    let mut picked = None;
    for (i, v) in values.iter().enumerate() {
        let at = r.min + Vec2::new((i / rows) as f32 * w, (i % rows) as f32 * line);
        let cell = Rect::from_min_size(at, Vec2::new(w, line)).intersect(r);
        let text = RichText::new(v).font(lk.font()).color(lk.color);
        let on = current.split('\r').any(|x| x == v);
        let clicked = if display == 3 {
            let mut b = on;
            ui.put(cell, egui::Checkbox::new(&mut b, text)).clicked()
        } else {
            ui.put(cell, egui::RadioButton::new(on, text)).clicked()
        };
        if clicked {
            picked = Some(if display == 3 {
                toggle(current, values, v)
            } else {
                v.clone()
            });
        }
    }
    picked
}

/// The checked values once `v` is toggled: value-list order first, then
/// the values that are not in the list, as they were.
pub(crate) fn toggle(current: &str, values: &[String], v: &str) -> String {
    let mut cur: Vec<&str> = current.split('\r').filter(|x| !x.is_empty()).collect();
    if cur.contains(&v) {
        cur.retain(|x| *x != v);
    } else {
        cur.push(v);
    }
    let mut out: Vec<&str> = Vec::new();
    for x in values.iter().map(String::as_str) {
        if cur.contains(&x) && !out.contains(&x) {
            out.push(x);
        }
    }
    out.extend(cur.iter().filter(|x| !values.iter().any(|v| v == *x)));
    out.join("\r")
}

/// The month grid of a drop-down calendar below `field`, on the month of
/// `day` (else today's). Returns the picked day number and whether the
/// pointer is over the calendar.
pub(crate) fn calendar(
    ui: &egui::Ui,
    id: egui::Id,
    field: Rect,
    day: Option<i64>,
) -> (Option<i64>, bool) {
    use calc::value::{civil, day_number, day_of_week};
    let today = calc::functions::builtin_get("CurrentDate").and_then(|v| v.to_date());
    let shown = id.with("month");
    let (mut y, mut m) = ui
        .data(|d| d.get_temp::<(i64, i64)>(shown))
        .unwrap_or_else(|| {
            let (y, m, _) = civil(day.or(today).unwrap_or_else(|| day_number(2026, 1, 1)));
            (y, m)
        });
    let mut picked = None;
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(field.left_bottom())
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("<").clicked() {
                        (y, m) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
                    }
                    ui.label(format!("{} {y}", MONTHS[(m - 1) as usize]));
                    if ui.small_button(">").clicked() {
                        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                    }
                });
                let first = day_number(y, m, 1);
                let next = if m == 12 {
                    day_number(y + 1, 1, 1)
                } else {
                    day_number(y, m + 1, 1)
                };
                egui::Grid::new(id.with("grid"))
                    .spacing([2.0, 2.0])
                    .show(ui, |ui| {
                        for d in ["S", "M", "T", "W", "T", "F", "S"] {
                            ui.label(d);
                        }
                        ui.end_row();
                        for _ in 1..day_of_week(first) {
                            ui.label("");
                        }
                        for d in first..next {
                            let label = (d - first + 1).to_string();
                            if ui.selectable_label(Some(d) == day, label).clicked() {
                                picked = Some(d);
                            }
                            if day_of_week(d) == 7 {
                                ui.end_row();
                            }
                        }
                    });
            })
        });
    ui.data_mut(|d| d.insert_temp(shown, (y, m)));
    if picked.is_some() {
        ui.data_mut(|d| d.remove_temp::<(i64, i64)>(shown));
    }
    (picked, area.response.contains_pointer())
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The calendar icon at the right of a drop-down calendar's content area.
pub(crate) fn calendar_icon(painter: &egui::Painter, r: Rect, color: egui::Color32) -> Rect {
    let b = Rect::from_center_size(
        r.right_center() - Vec2::new(8.0, 0.0),
        Vec2::new(11.0, 10.0),
    );
    let s = egui::Stroke::new(1.0, color);
    painter.rect_stroke(b, 1.0, s, egui::StrokeKind::Inside);
    painter.line_segment(
        [
            b.left_top() + Vec2::new(0.0, 3.0),
            b.right_top() + Vec2::new(0.0, 3.0),
        ],
        s,
    );
    b.expand(3.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn toggle_keeps_value_list_order() {
        let vl: Vec<String> = ["Red", "Green", "Blue"].map(String::from).to_vec();
        assert_eq!(super::toggle("", &vl, "Blue"), "Blue");
        assert_eq!(super::toggle("Blue", &vl, "Red"), "Red\rBlue");
        assert_eq!(super::toggle("Red\rBlue", &vl, "Red"), "Blue");
        assert_eq!(super::toggle("x\rBlue", &vl, "Green"), "Green\rBlue\rx");
    }
}
