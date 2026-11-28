//! Calculation functions a running script answers for the window
//! ([`calc::Context::host_function`]): GetLayoutObjectAttribute,
//! GetPersistentData and ListPersistentDataIDs (Claris help).

use crate::Window;
use calc::Value;
use engine::Database;
use fmp12::layouts::{Layout, LayoutObject};

/// What the functions see: the window, its record and the persistent data
/// of Configure Persistent Data (name, instance id, value; oldest first).
pub(crate) struct Seen<'a> {
    pub db: &'a Database,
    pub window: &'a Window,
    pub table: u32,
    pub to: u32,
    pub record: Option<usize>,
    pub persistent: &'a [(String, String, Value)],
}

impl Seen<'_> {
    pub fn call(&self, name: &str, a: &[Value]) -> Option<Value> {
        let text = |i: usize| a.get(i).map(Value::to_text).unwrap_or_default();
        match name {
            // No entry with that name and instance id: "?".
            "GetPersistentData" => Some(
                self.persistent
                    .iter()
                    .find(|(n, id, _)| *n == text(0) && *id == text(1))
                    .map_or_else(|| Value::text("?"), |e| e.2.clone()),
            ),
            "ListPersistentDataIDs" => Some(Value::Text(
                self.persistent
                    .iter()
                    .filter(|(n, ..)| *n == text(0))
                    .map(|(_, id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join("\r"),
            )),
            "GetLayoutObjectAttribute" => {
                let n = |i: usize| {
                    a.get(i)
                        .and_then(Value::to_num)
                        .map_or(0, |n| calc::num::to_f64(&n).max(0.0) as usize)
                };
                Some(self.attribute(&text(0), &text(1), n(2), n(3)))
            }
            _ => None,
        }
    }

    fn layout(&self) -> Option<&Layout> {
        self.db.layouts.iter().find(|l| l.id == self.window.layout)
    }

    /// GetLayoutObjectAttribute ( objectName ; attributeName ;
    /// repetition ; portalRow ); empty for an unknown object or attribute
    /// (FileMaker 26 for an unknown object).
    // ponytail: rotation is 0 (not decoded); the window's chrome is taken
    // to sit above the content; a graphic's source, a chart's content and
    // a web viewer's content are empty; the front tab panel is the stored
    // one (the window's tab choice is not tracked).
    fn attribute(&self, name: &str, attr: &str, rep: usize, row: usize) -> Value {
        let Some(l) = self.layout() else {
            return Value::empty();
        };
        let Some(o) = l
            .objects
            .iter()
            .find(|o| !o.name.is_empty() && o.name == name)
        else {
            return Value::empty();
        };
        let b = l.absolute(o);
        let w = self.window;
        let (dx, dy) = match w.host.content {
            Some((_, ch)) => (w.left, w.top + (w.height - ch).max(0.0)),
            None => (w.left, w.top),
        };
        let pt =
            |x: f64| calc::num::from_f64(x, 2).map_or_else(String::new, |n| calc::num::format(&n));
        let (left, top, right, bottom) = (b.left + dx, b.top + dy, b.right + dx, b.bottom + dy);
        let bool = |b: bool| Value::int(b as i64);
        Value::Text(match attr.to_ascii_lowercase().as_str() {
            "objecttype" => kind(o).into(),
            "bounds" => format!("{} {} {} {} 0", pt(left), pt(top), pt(right), pt(bottom)),
            "left" => pt(left),
            "top" => pt(top),
            "right" => pt(right),
            "bottom" => pt(bottom),
            "width" => pt(b.right - b.left),
            "height" => pt(b.bottom - b.top),
            "rotation" => "0".into(),
            "startpoint" => format!("{} {}", pt(left), pt(top)),
            "endpoint" => format!("{} {}", pt(right), pt(bottom)),
            "hasfocus" => return bool(self.focus(o)),
            "containsfocus" => return bool(self.descendants(l, o).any(|x| self.focus(x))),
            "isfrontpanel" => return bool(front(l, o)),
            "isactive" if matches!(o.kind, 12 | 17) => return bool(front(l, o)),
            "isactive" => return bool(self.focus(o)),
            "isobjecthidden" => return bool(self.hidden(l, o)),
            "source" => self.source(l, o),
            "content" => return self.content(l, o, rep, row),
            "enclosingobject" => enclosing(l, o).map(|e| e.name.clone()).unwrap_or_default(),
            "containedobjects" => l
                .objects
                .iter()
                .filter(|x| !x.name.is_empty() && enclosing(l, x).is_some_and(|e| e.id == o.id))
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join("\r"),
            _ => String::new(),
        })
    }

    /// The object is the active field (or the Go to Object target).
    fn focus(&self, o: &LayoutObject) -> bool {
        let w = self.window;
        if !w.object.is_empty() {
            return w.object == o.name;
        }
        match (w.field, o.field) {
            (Some(f), Some((t, x, _))) => (t & 0xFFF) as u32 == f.to && x as u32 == f.field,
            _ => false,
        }
    }

    /// `o` and every object inside it.
    fn descendants<'l>(
        &self,
        l: &'l Layout,
        o: &'l LayoutObject,
    ) -> impl Iterator<Item = &'l LayoutObject> {
        let id = o.id;
        l.objects.iter().filter(move |x| {
            let mut p = Some(x.id);
            while let Some(pid) = p {
                if pid == id {
                    return true;
                }
                p = l
                    .objects
                    .iter()
                    .find(|y| y.id == pid)
                    .and_then(|y| y.parent);
            }
            false
        })
    }

    /// Beyond the layout's right edge, or "Hide object when" is true.
    fn hidden(&self, l: &Layout, o: &LayoutObject) -> bool {
        if l.width > 0.0 && l.absolute(o).left >= l.width && o.kind != 15 {
            return true;
        }
        o.hide.as_ref().is_some_and(|h| {
            self.db
                .eval_tokens(h, self.table, self.to, self.record)
                .truth()
        })
    }

    fn source(&self, l: &Layout, o: &LayoutObject) -> String {
        let to_name = |to: u64| {
            self.db
                .occurrences
                .iter()
                .find(|x| x.id == (to & 0xFFF) as u32)
                .map(|x| x.name.clone())
                .unwrap_or_default()
        };
        if let Some((to, f, _)) = o.field {
            let field = self
                .db
                .table_of((to & 0xFFF) as u32)
                .and_then(|t| t.field(f as u32))
                .map(|f| f.name.clone())
                .unwrap_or_default();
            return format!("{}::{field}", to_name(to));
        }
        if let Some((to, ..)) = o.portal {
            return to_name(to);
        }
        if let Some(u) = &o.url {
            return self.eval(u).to_text();
        }
        match (&o.text, o.kind) {
            (Some(t), 2) => self.db.merge_names(t, (l.table_occurrence & 0xFFF) as u32),
            _ => String::new(),
        }
    }

    fn eval(&self, tokens: &[u8]) -> Value {
        self.db
            .eval_tokens(tokens, self.table, self.to, self.record)
    }

    /// A field's value (repetition, portal row); a text object's or a
    /// button's text with its merge fields filled in.
    fn content(&self, l: &Layout, o: &LayoutObject, rep: usize, row: usize) -> Value {
        if let Some((to, f, _)) = o.field {
            return self
                .value(to, f, rep.max(1), row)
                .map_or_else(Value::empty, |v| Value::Text(v.to_text()));
        }
        if let Some(label) = &o.label {
            return Value::Text(self.eval(label).to_text());
        }
        if matches!(o.kind, 2 | 10 | 14) {
            let t = o
                .text
                .clone()
                .or_else(|| self.descendants(l, o).find(|x| x.kind == 2)?.text.clone());
            return Value::Text(self.merged(&t.unwrap_or_default(), l));
        }
        Value::empty()
    }

    /// Field `f` of occurrence `to` for the window's record: the record's
    /// own, or related record `row` (1 when 0).
    fn value(&self, to: u64, f: u64, rep: usize, row: usize) -> Option<Value> {
        let (to, f) = ((to & 0xFFF) as u32, f as u32);
        let r = self.record?;
        let t = self.db.table_of(to)?;
        let at = if to == self.to || t.id == self.table && to == 0 {
            r
        } else {
            *self.db.related(self.to, r, to).get(row.max(1) - 1)?
        };
        Some(self.db.value_rep(t.id, at, f, rep as u32))
    }

    fn merged(&self, text: &str, l: &Layout) -> String {
        let layout_to = (l.table_occurrence & 0xFFF) as u32;
        let mut out = String::new();
        let mut rest = text;
        while let Some(a) = rest.find("<<") {
            let Some(b) = rest[a..].find(">>") else { break };
            out.push_str(&rest[..a]);
            let inner = &rest[a + 2..a + b];
            match self.db.merge_field(inner, layout_to) {
                Some((to, f)) => {
                    let v = self.value(to as u64, f as u64, 1, 1);
                    out.push_str(&v.map(|v| v.to_text()).unwrap_or_default());
                }
                None => out.push_str(&rest[a..a + b + 2]),
            }
            rest = &rest[a + b + 2..];
        }
        out.push_str(rest);
        out
    }
}

/// The object type, as FileMaker names it in English.
fn kind(o: &LayoutObject) -> &'static str {
    match o.kind {
        1 => "field",
        2 => "text",
        3 => "graphic",
        4 => "line",
        5 => "rectangle",
        6 => "rounded rectangle",
        7 => "oval",
        8 => "group",
        9 => "portal",
        10 => "button",
        11 => "tab control",
        12 => "tab panel",
        13 if o.chart.is_some() => "chart",
        13 => "web viewer",
        14 => "popover button",
        15 => "popover",
        16 => "slide control",
        17 => "slide panel",
        18 => "button bar",
        _ => "unknown",
    }
}

/// A tab or slide panel in front of its control.
fn front(l: &Layout, o: &LayoutObject) -> bool {
    let parent = o.parent.and_then(|p| l.objects.iter().find(|x| x.id == p));
    matches!(o.kind, 12 | 17) && parent.is_some_and(|p| p.active_child == Some(o.id))
}

/// The nearest named object around `o`.
fn enclosing<'l>(l: &'l Layout, o: &LayoutObject) -> Option<&'l LayoutObject> {
    let mut p = o.parent;
    while let Some(pid) = p {
        let x = l.objects.iter().find(|x| x.id == pid)?;
        if !x.name.is_empty() {
            return Some(x);
        }
        p = x.parent;
    }
    None
}
