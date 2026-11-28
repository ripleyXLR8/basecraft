//! Per-record state of a layout object, shared by Browse mode, Preview mode
//! and PDF output: "hide object when", conditional formatting and data
//! formatting.

use calc::Value;
use engine::Database;
use fmp12::layouts::{Condition, LayoutObject, op};
use fmp12::pb::{Field, MsgBuf};

/// Where a calculation of the object is evaluated: table, layout
/// occurrence, record.
#[derive(Clone, Copy)]
pub struct Context {
    pub table: u32,
    pub to: u32,
    pub record: Option<usize>,
}

/// "Hide object when": the calculation is true for the record (`Self`:
/// the object's value).
pub fn hidden(db: &Database, o: &LayoutObject, c: Context) -> bool {
    o.hide.as_ref().is_some_and(|h| {
        db.eval_with_self(h, c.table, c.to, c.record, &self_value(db, o, c), None)
            .truth()
    })
}

/// `Self` of an object: its field's value for the record, else its text.
/// ponytail: a related field (another table) reads as empty; callers that
/// know the related record pass the value to [`conditional_style`].
fn self_value(db: &Database, o: &LayoutObject, c: Context) -> Value {
    match (o.field, c.record) {
        (Some((to, f, rep)), Some(r)) => {
            if db.table_of((to & 0xFFF) as u32).map(|t| t.id) == Some(c.table) {
                db.value_rep(c.table, r, f as u32, rep.max(1) as u32)
            } else {
                Value::empty()
            }
        }
        (Some(_), None) => Value::empty(),
        _ => Value::text(o.text.clone().unwrap_or_default()),
    }
}

/// A compared value of a condition: text in quotes, a number, else the text
/// itself.
/// ponytail: FileMaker evaluates these as calculations; literals cover the
/// values its dialog writes.
fn operand(s: &str) -> Value {
    let t = s.trim();
    if let Some(q) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return Value::text(q.replace("\"\"", "\""));
    }
    calc::value::text_to_num(t)
        .filter(|_| !t.is_empty())
        .map_or_else(|| Value::text(t), Value::num)
}

fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.to_num(), b.to_num()) {
        (Some(x), Some(y)) if !matches!(a, Value::Text(_)) || !matches!(b, Value::Text(_)) => {
            x.cmp(&y)
        }
        _ => a.to_text().to_lowercase().cmp(&b.to_text().to_lowercase()),
    }
}

/// Today as a day number (UTC date).
fn today() -> Option<i64> {
    let secs = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    // 1970-01-01 as a FileMaker day number.
    Some(calc::value::day_number(1970, 1, 1) + secs.div_euclid(86_400))
}

/// Whether condition `k` holds for the object's `value`.
pub fn holds(db: &Database, k: &Condition, value: &Value, c: Context) -> bool {
    match k.op {
        op::FORMULA => k.calc.as_ref().is_some_and(|t| {
            db.eval_with_self(t, c.table, c.to, c.record, value, None)
                .truth()
        }),
        _ => compares(k, value),
    }
}

/// A value comparison ("Value is …") of condition `k`, evaluated from its
/// operator and values (the formula FileMaker stores for it compares
/// `Self`, which is the same test).
pub fn compares(k: &Condition, value: &Value) -> bool {
    use std::cmp::Ordering::*;
    let (a, b) = (operand(&k.value1), operand(&k.value2));
    let text = value.to_text().to_lowercase();
    let at = a.to_text().to_lowercase();
    match k.op {
        op::BETWEEN => compare(value, &a) != Less && compare(value, &b) != Greater,
        op::NOT_BETWEEN => compare(value, &a) == Less || compare(value, &b) == Greater,
        op::EQUAL => compare(value, &a) == Equal,
        op::NOT_EQUAL => compare(value, &a) != Equal,
        op::GREATER => compare(value, &a) == Greater,
        op::GREATER_EQUAL => compare(value, &a) != Less,
        op::LESS => compare(value, &a) == Less,
        op::LESS_EQUAL => compare(value, &a) != Greater,
        op::CONTAINING => text.contains(&at),
        op::NOT_CONTAINING => !text.contains(&at),
        op::BEGINNING => text.starts_with(&at),
        op::ENDING => text.ends_with(&at),
        op::EMPTY => value.is_empty(),
        // Date tests against today, in days.
        op::BEFORE_TODAY..=op::LESS_DAYS_BEFORE => {
            let (Some(d), Some(today)) = (value.to_date(), today()) else {
                return false;
            };
            let n = a.to_num().map_or(0, |x| {
                use bigdecimal::ToPrimitive;
                x.to_i64().unwrap_or(0)
            });
            match k.op {
                op::BEFORE_TODAY => d < today,
                op::TODAY => d == today,
                op::AFTER_TODAY => d > today,
                op::MORE_DAYS_AFTER => d > today + n,
                op::MORE_DAYS_BEFORE => d < today - n,
                op::LESS_DAYS_AFTER => d < today + n,
                _ => d > today - n,
            }
        }
        _ => false,
    }
}

/// The object's style with the local rules of every condition that holds
/// added after its own (a later rule overrides an earlier one).
pub fn conditional_style(db: &Database, o: &LayoutObject, value: &Value, c: Context) -> Vec<u8> {
    let extra: Vec<Vec<u8>> = o
        .conditions
        .iter()
        .filter(|k| holds(db, k, value, c))
        .filter_map(|k| MsgBuf::parse(&k.style())?.bytes(1).map(<[u8]>::to_vec))
        .collect();
    if extra.is_empty() {
        return o.style_raw.clone();
    }
    let mut m = MsgBuf::parse(&o.style_raw).unwrap_or_default();
    let mut rules = m.bytes(1).map(<[u8]>::to_vec).unwrap_or_default();
    for e in extra {
        rules.extend(e);
    }
    m.set(1, Field::Bytes(rules));
    m.encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_conditions() {
        let k = |op, v1: &str, v2: &str| Condition {
            op,
            value1: v1.into(),
            value2: v2.into(),
            ..Default::default()
        };
        let hello = Value::text("hello");
        assert!(compares(&k(op::EQUAL, "\"Hello\"", ""), &hello));
        assert!(compares(&k(op::BETWEEN, "\"he\"", "\"z\""), &hello));
        assert!(!compares(&k(op::LESS, "\"he\"", ""), &hello));
        assert!(compares(&k(op::BEGINNING, "\"he\"", ""), &hello));
        assert!(!compares(&k(op::EMPTY, "", ""), &hello));
        let ten = Value::int(10);
        assert!(compares(&k(op::GREATER, "9", ""), &ten));
        assert!(!compares(&k(op::GREATER, "11", ""), &ten));
    }
}
