//! JSON functions. Objects keep their keys sorted (FileMaker's JSON library
//! does too); compact output has no spaces, JSONFormatElements uses the
//! tab-indented "styled" layout.
//!
//! Parse errors are not calculation errors: FileMaker returns them as text
//! starting with `?`.

use super::txt;
use crate::ast::Expr;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num;
use crate::value::Value;
use serde_json::{Map, Number, Value as J};
use std::str::FromStr;

const RAW: i64 = 0;
const STRING: i64 = 1;
const NUMBER: i64 = 2;
const OBJECT: i64 = 3;
const ARRAY: i64 = 4;
const BOOLEAN: i64 = 5;
const NULL: i64 = 6;

// ponytail: messages are serde_json's, not the exact FileMaker wording.
fn err_text(msg: impl std::fmt::Display) -> Value {
    Value::Text(format!("? {msg}"))
}

fn parse(s: &str) -> Result<J, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
    Last,
    Append,
}

/// `a.b[2]['c.d'][:][+]`
fn parse_path(p: &str) -> Result<Vec<Seg>, String> {
    let c: Vec<char> = p.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '.' => i += 1,
            '[' => {
                let close = if matches!(c.get(i + 1), Some('\'' | '"')) {
                    let q = c[i + 1];
                    let end = (i + 2..c.len())
                        .find(|&j| c[j] == q)
                        .ok_or("unterminated quoted key")?;
                    out.push(Seg::Key(c[i + 2..end].iter().collect()));
                    end + 1
                } else {
                    let end = (i..c.len()).find(|&j| c[j] == ']').ok_or("missing ]")?;
                    let inner: String = c[i + 1..end].iter().collect();
                    out.push(match inner.trim() {
                        ":" => Seg::Last,
                        "+" => Seg::Append,
                        n => Seg::Index(n.parse().map_err(|_| format!("invalid index [{n}]"))?),
                    });
                    end
                };
                if c.get(close) != Some(&']') {
                    return Err("missing ]".into());
                }
                i = close + 1;
            }
            _ => {
                let start = i;
                while i < c.len() && c[i] != '.' && c[i] != '[' {
                    i += 1;
                }
                out.push(Seg::Key(c[start..i].iter().collect()));
            }
        }
    }
    Ok(out)
}

fn index(node: &J, seg: &Seg) -> Option<usize> {
    let J::Array(a) = node else { return None };
    match seg {
        Seg::Index(i) => Some(*i),
        Seg::Key(k) => k.parse().ok(),
        Seg::Last => a.len().checked_sub(1),
        Seg::Append => None,
    }
}

fn get<'a>(mut node: &'a J, path: &[Seg]) -> Option<&'a J> {
    for seg in path {
        node = match (node, seg) {
            (J::Object(m), Seg::Key(k)) => m.get(k)?,
            (J::Array(a), s) => a.get(index(node, s)?)?,
            _ => return None,
        };
    }
    Some(node)
}

fn set(node: &mut J, path: &[Seg], value: J) -> Result<(), String> {
    let Some((seg, rest)) = path.split_first() else {
        *node = value;
        return Ok(());
    };
    if node.is_null() {
        *node = match seg {
            Seg::Key(_) => J::Object(Map::new()),
            _ => J::Array(vec![]),
        };
    }
    match node {
        J::Object(m) => match seg {
            Seg::Key(k) => set(m.entry(k.clone()).or_insert(J::Null), rest, value),
            _ => Err("array index used on an object".into()),
        },
        J::Array(a) => {
            let i = match seg {
                Seg::Append => a.len(),
                Seg::Last => a.len().saturating_sub(1),
                Seg::Index(i) => *i,
                Seg::Key(k) => k.parse().map_err(|_| format!("key {k} used on an array"))?,
            };
            if a.len() <= i {
                a.resize(i + 1, J::Null);
            }
            set(&mut a[i], rest, value)
        }
        _ => Err("cannot set an element inside a scalar".into()),
    }
}

fn delete(node: &mut J, path: &[Seg]) {
    let Some((last, parent)) = path.split_last() else {
        *node = J::Null;
        return;
    };
    let mut n = node;
    for seg in parent {
        n = match n {
            J::Object(m) => match seg {
                Seg::Key(k) => match m.get_mut(k) {
                    Some(c) => c,
                    None => return,
                },
                _ => return,
            },
            J::Array(_) => {
                let Some(i) = index(n, seg) else { return };
                match n.get_mut(i) {
                    Some(c) => c,
                    None => return,
                }
            }
            _ => return,
        };
    }
    match n {
        J::Object(m) => {
            if let Seg::Key(k) = last {
                m.remove(k);
            }
        }
        J::Array(_) => {
            if let Some(i) = index(n, last)
                && let J::Array(a) = n
                && i < a.len()
            {
                a.remove(i);
            }
        }
        _ => {}
    }
}

fn number_text(n: &Number) -> String {
    let s = n.to_string();
    num::parse(&s).map(|d| num::format(&d)).unwrap_or(s)
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn scalar(j: &J) -> Option<String> {
    Some(match j {
        J::Null => "null".into(),
        J::Bool(b) => b.to_string(),
        J::Number(n) => number_text(n),
        J::String(s) => {
            let mut o = String::new();
            quote(s, &mut o);
            o
        }
        _ => return None,
    })
}

pub(crate) fn compact(j: &J) -> String {
    let mut out = String::new();
    write_compact(j, &mut out);
    out
}

fn write_compact(j: &J, out: &mut String) {
    match j {
        J::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact(v, out);
            }
            out.push(']');
        }
        J::Object(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(k, out);
                out.push(':');
                write_compact(v, out);
            }
            out.push('}');
        }
        s => out.push_str(&scalar(s).unwrap()),
    }
}

/// JsonCpp's StyledStreamWriter layout with tab indentation.
// ponytail: line ends are CR (FileMaker's paragraph separator); verify
// against FileMaker whether it emits LF.
struct Styled {
    out: String,
    indent: String,
    indented: bool,
}

const RIGHT_MARGIN: usize = 74;
const EOL: &str = "\r";

impl Styled {
    fn write_indent(&mut self) {
        self.out.push_str(EOL);
        self.out.push_str(&self.indent);
    }

    fn with_indent(&mut self, s: &str) {
        if !self.indented {
            self.write_indent();
        }
        self.out.push_str(s);
        self.indented = false;
    }

    fn value(&mut self, j: &J) {
        match j {
            J::Object(m) if !m.is_empty() => {
                self.with_indent("{");
                self.indent.push('\t');
                for (i, (k, v)) in m.iter().enumerate() {
                    let mut key = String::new();
                    quote(k, &mut key);
                    self.with_indent(&key);
                    self.out.push_str(" : ");
                    self.value(v);
                    if i + 1 < m.len() {
                        self.out.push(',');
                    }
                }
                self.indent.pop();
                self.with_indent("}");
            }
            J::Object(_) => self.out.push_str("{}"),
            J::Array(a) if a.is_empty() => self.out.push_str("[]"),
            J::Array(a) => {
                let nested = a.iter().any(|c| match c {
                    J::Array(x) => !x.is_empty(),
                    J::Object(x) => !x.is_empty(),
                    _ => false,
                });
                let flat: Option<Vec<String>> =
                    (!nested && a.len() * 3 < RIGHT_MARGIN).then(|| {
                        a.iter()
                            .map(|c| scalar(c).unwrap_or_else(|| compact(c)))
                            .collect()
                    });
                let len = flat.as_ref().map(|f| {
                    4 + (a.len() - 1) * 2 + f.iter().map(|s| s.chars().count()).sum::<usize>()
                });
                match (flat, len) {
                    (Some(f), Some(len)) if len < RIGHT_MARGIN => {
                        self.out.push_str("[ ");
                        self.out.push_str(&f.join(", "));
                        self.out.push_str(" ]");
                    }
                    (flat, _) => {
                        self.with_indent("[");
                        self.indent.push('\t');
                        for (i, c) in a.iter().enumerate() {
                            match &flat {
                                Some(f) => self.with_indent(&f[i]),
                                None => {
                                    if !self.indented {
                                        self.write_indent();
                                    }
                                    self.indented = true;
                                    self.value(c);
                                    self.indented = false;
                                }
                            }
                            if i + 1 < a.len() {
                                self.out.push(',');
                            }
                        }
                        self.indent.pop();
                        self.with_indent("]");
                    }
                }
            }
            s => self.out.push_str(&scalar(s).unwrap()),
        }
    }
}

fn styled(j: &J) -> String {
    let mut w = Styled {
        out: String::new(),
        indent: String::new(),
        indented: true,
    };
    w.value(j);
    w.out
}

/// The FileMaker value of an element: strings unquoted, numbers and
/// booleans as numbers, null as empty, containers as compact JSON.
fn element(j: &J) -> Value {
    match j {
        J::Null => Value::empty(),
        J::Bool(b) => Value::bool(*b),
        J::Number(n) => num::parse(&n.to_string())
            .map(Value::Number)
            .unwrap_or_else(Value::empty),
        J::String(s) => Value::text(s.clone()),
        j => Value::Text(compact(j)),
    }
}

/// Parses `json` and runs `f` on the element at `path`; parse errors become
/// `? message` text.
fn at(a: &[Value], f: impl FnOnce(Option<&J>) -> Value) -> R {
    let root = match parse(&txt(a, 0)) {
        Ok(j) => j,
        Err(e) => return Ok(err_text(e)),
    };
    let path = match parse_path(&txt(a, 1)) {
        Ok(p) => p,
        Err(e) => return Ok(err_text(e)),
    };
    Ok(f(get(&root, &path)))
}

pub fn get_element(_: &mut Evaluator, a: &[Value]) -> R {
    at(a, |j| j.map_or_else(Value::empty, element))
}

pub fn get_element_type(_: &mut Evaluator, a: &[Value]) -> R {
    at(a, |j| {
        Value::int(match j {
            None => 0,
            Some(J::String(_)) => STRING,
            Some(J::Number(_)) => NUMBER,
            Some(J::Object(_)) => OBJECT,
            Some(J::Array(_)) => ARRAY,
            Some(J::Bool(_)) => BOOLEAN,
            Some(J::Null) => NULL,
        })
    })
}

pub fn list_keys(_: &mut Evaluator, a: &[Value]) -> R {
    at(a, |j| {
        let keys: Vec<String> = match j {
            Some(J::Object(m)) => m.keys().cloned().collect(),
            Some(J::Array(v)) => (0..v.len()).map(|i| i.to_string()).collect(),
            _ => vec![],
        };
        Value::Text(keys.join("\r"))
    })
}

pub fn list_values(_: &mut Evaluator, a: &[Value]) -> R {
    at(a, |j| {
        let text = |c: &J| match c {
            J::Bool(b) => b.to_string(),
            c => element(c).to_text(),
        };
        let vals: Vec<String> = match j {
            Some(J::Object(m)) => m.values().map(text).collect(),
            Some(J::Array(v)) => v.iter().map(text).collect(),
            _ => vec![],
        };
        Value::Text(vals.join("\r"))
    })
}

pub fn delete_element(_: &mut Evaluator, a: &[Value]) -> R {
    let mut root = match parse(&txt(a, 0)) {
        Ok(j) => j,
        Err(e) => return Ok(err_text(e)),
    };
    match parse_path(&txt(a, 1)) {
        Ok(p) => delete(&mut root, &p),
        Err(e) => return Ok(err_text(e)),
    }
    Ok(Value::Text(compact(&root)))
}

pub fn format_elements(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(match parse(&txt(a, 0)) {
        Ok(j) => Value::Text(styled(&j)),
        Err(e) => err_text(e),
    })
}

/// Converts a calculation value to JSON according to a JSONString... type.
fn typed(v: &Value, ty: i64) -> Result<J, String> {
    Ok(match ty {
        STRING => J::String(v.to_text()),
        NUMBER => {
            let n = num::format(&v.num_or_zero());
            J::Number(Number::from_str(&n).map_err(|e| e.to_string())?)
        }
        BOOLEAN => J::Bool(match v.to_text().trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => v.truth(),
        }),
        NULL => J::Null,
        OBJECT | ARRAY if v.is_empty() => {
            if ty == OBJECT {
                J::Object(Map::new())
            } else {
                J::Array(vec![])
            }
        }
        OBJECT | ARRAY | RAW => parse(&v.to_text())?,
        _ => return Err(format!("invalid JSON type {ty}")),
    })
}

pub fn set_element(ev: &mut Evaluator, a: &[Expr]) -> R {
    let triples: Vec<&[Expr]> = if matches!(a[1], Expr::List(_)) {
        a[1..]
            .iter()
            .map(|e| match e {
                Expr::List(t) if t.len() == 3 => Ok(t.as_slice()),
                _ => Err(EvalError::new(
                    code::TOO_FEW_PARAMS,
                    "JSONSetElement lists need [key ; value ; type]",
                )),
            })
            .collect::<Result<_, _>>()?
    } else if a.len() == 4 {
        vec![&a[1..4]]
    } else {
        return Err(EvalError::new(
            code::TOO_FEW_PARAMS,
            "JSONSetElement needs key, value and type",
        ));
    };
    let src = ev.eval(&a[0])?.to_text();
    let mut root = if src.trim().is_empty() {
        J::Null
    } else {
        match parse(&src) {
            Ok(j) => j,
            Err(e) => return Ok(err_text(e)),
        }
    };
    for t in triples {
        let path = ev.eval(&t[0])?.to_text();
        let value = ev.eval(&t[1])?;
        let ty = crate::eval::to_index(&ev.eval(&t[2])?)?;
        let r =
            parse_path(&path).and_then(|p| typed(&value, ty).and_then(|v| set(&mut root, &p, v)));
        if let Err(e) = r {
            return Ok(err_text(e));
        }
    }
    if root.is_null() {
        root = J::Object(Map::new());
    }
    // JSONParsedState reports JSONSetElement's result as parsed.
    Ok(Value::Json {
        text: compact(&root),
        state: type_code(&root),
    })
}

fn type_code(j: &J) -> i64 {
    match j {
        J::String(_) => STRING,
        J::Number(_) => NUMBER,
        J::Object(_) => OBJECT,
        J::Array(_) => ARRAY,
        J::Bool(_) => BOOLEAN,
        J::Null => NULL,
    }
}

/// JSONParse: the text unchanged, marked as parsed; invalid JSON gives the
/// `?` error text, marked invalid. Only an object or an array parses: the
/// documented JSONParse ( Pi ) is invalid.
// ponytail: no binary cache; every JSON function reparses its text.
pub fn json_parse(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let parsed = parse(&s).and_then(|j| match j {
        J::Object(_) | J::Array(_) => Ok(j),
        _ => Err("JSONParse needs an object or an array".to_string()),
    });
    Ok(match parsed {
        Ok(j) => Value::Json {
            text: s,
            state: type_code(&j),
        },
        Err(e) => Value::Json {
            text: err_text(e).to_text(),
            state: -1,
        },
    })
}

/// 0 unparsed text, -1 parsed but invalid, else the JSON type (1..6).
pub fn json_parsed_state(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(match &a[0] {
        Value::Json { state, .. } => *state,
        _ => 0,
    }))
}

/// A calculation value as the JSON it stands for: JSON text when it parses
/// (so 1 matches the number 1 and "\"1\"" the string), else a string.
// ponytail: JSONValueExists & co. are undocumented FileMaker 26 functions;
// this matching rule and their results below are our reading of the
// signatures. Verify against FileMaker.
fn as_json(v: &Value) -> J {
    match v {
        Value::Number(n) => Number::from_str(&num::format(n))
            .map(J::Number)
            .unwrap_or(J::Null),
        v => {
            let s = v.to_text();
            parse(&s).unwrap_or(J::String(s))
        }
    }
}

/// The values of the array or object at the path, compared by their
/// compact text (numbers normalised).
fn members(j: Option<&J>) -> Vec<String> {
    match j {
        Some(J::Array(a)) => a.iter().map(compact).collect(),
        Some(J::Object(m)) => m.values().map(compact).collect(),
        _ => vec![],
    }
}

fn count_of(a: &[Value]) -> R {
    let want = compact(&as_json(&a[2]));
    at(a, |j| {
        Value::int(members(j).iter().filter(|m| **m == want).count() as i64)
    })
}

pub fn value_count(_: &mut Evaluator, a: &[Value]) -> R {
    count_of(a)
}

pub fn value_exists(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(match count_of(a)? {
        Value::Number(n) => Value::bool(!bigdecimal::Zero::is_zero(&n)),
        v => v,
    })
}

/// The values of a JSON array argument (`jsonValuesArray...`).
fn wanted(v: &Value) -> Result<Vec<String>, String> {
    match parse(&v.to_text())? {
        J::Array(a) => Ok(a.iter().map(compact).collect()),
        j => Ok(vec![compact(&j)]),
    }
}

pub fn any_values_exist(_: &mut Evaluator, a: &[Value]) -> R {
    let want = match wanted(&a[2]) {
        Ok(w) => w,
        Err(e) => return Ok(err_text(e)),
    };
    at(a, |j| {
        Value::bool(members(j).iter().any(|m| want.contains(m)))
    })
}

/// Rewrites the array or object at the path with `keep`, returning the
/// whole document.
fn filter_at(a: &[Value], mut keep: impl FnMut(&str) -> bool) -> R {
    let mut root = match parse(&txt(a, 0)) {
        Ok(j) => j,
        Err(e) => return Ok(err_text(e)),
    };
    let path = match parse_path(&txt(a, 1)) {
        Ok(p) => p,
        Err(e) => return Ok(err_text(e)),
    };
    let mut node = &mut root;
    for seg in &path {
        let next = match node {
            J::Object(m) => match seg {
                Seg::Key(k) => m.get_mut(k),
                _ => None,
            },
            J::Array(_) => {
                let i = index(node, seg);
                i.and_then(|i| node.get_mut(i))
            }
            _ => None,
        };
        match next {
            Some(n) => node = n,
            None => return Ok(Value::Text(compact(&root_copy(a)))),
        }
    }
    match node {
        J::Array(v) => v.retain(|x| keep(&compact(x))),
        J::Object(m) => m.retain(|_, x| keep(&compact(x))),
        _ => {}
    }
    Ok(Value::Text(compact(&root)))
}

fn root_copy(a: &[Value]) -> J {
    parse(&txt(a, 0)).unwrap_or(J::Null)
}

pub fn remove_values(_: &mut Evaluator, a: &[Value]) -> R {
    let drop = match wanted(&a[2]) {
        Ok(w) => w,
        Err(e) => return Ok(err_text(e)),
    };
    filter_at(a, |m| !drop.iter().any(|d| d == m))
}

/// Keeps the first of equal values.
pub fn unique_values(_: &mut Evaluator, a: &[Value]) -> R {
    let mut seen: Vec<String> = vec![];
    filter_at(a, |m| {
        let new = !seen.iter().any(|s| s == m);
        if new {
            seen.push(m.to_string());
        }
        new
    })
}

pub fn make_array(_: &mut Evaluator, a: &[Value]) -> R {
    let list = txt(a, 0);
    let sep = txt(a, 1);
    let ty = super::int(a, 2)?;
    let items: Vec<&str> = if sep.is_empty() || sep == "\r" {
        super::text::split_values(&list)
    } else if list.is_empty() {
        vec![]
    } else {
        list.split(sep.as_str()).collect()
    };
    let mut out = vec![];
    for i in items {
        match typed(&Value::text(i), ty) {
            Ok(j) => out.push(j),
            Err(e) => return Ok(err_text(e)),
        }
    }
    Ok(Value::Text(compact(&J::Array(out))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            parse_path("a.b[2]['c.d'][:][+]").unwrap(),
            vec![
                Seg::Key("a".into()),
                Seg::Key("b".into()),
                Seg::Index(2),
                Seg::Key("c.d".into()),
                Seg::Last,
                Seg::Append
            ]
        );
        assert_eq!(parse_path("bakery.product[1]name").unwrap().len(), 4);
    }

    #[test]
    fn styled_layout() {
        let j = parse(r#"{"a":{"lnk":false,"id":12}}"#).unwrap();
        assert_eq!(
            styled(&j),
            "{\r\t\"a\" : \r\t{\r\t\t\"id\" : 12,\r\t\t\"lnk\" : false\r\t}\r}"
        );
        let j = parse(r#"{"a":[1,2,3],"b":[],"c":[{"x":1}]}"#).unwrap();
        assert_eq!(
            styled(&j),
            "{\r\t\"a\" : [ 1, 2, 3 ],\r\t\"b\" : [],\r\t\"c\" : \r\t[\r\t\t{\r\t\t\t\"x\" : 1\r\t\t}\r\t]\r}"
        );
    }
}
