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

const STRING: i64 = 1;
const NUMBER: i64 = 2;
const OBJECT: i64 = 3;
const ARRAY: i64 = 4;
const BOOLEAN: i64 = 5;
const NULL: i64 = 6;

fn err_text(msg: impl std::fmt::Display) -> Value {
    Value::Text(format!("? {msg}"))
}

const NOT_A_DOCUMENT: &str = "A valid JSON document must be either an array or an object value.";
const VALUE_EXPECTED: &str = "Syntax error: value, object or array expected.";

/// Parses a JSON document, which must be an object or an array (FileMaker's
/// JsonCpp runs in strict-root mode). Errors use JsonCpp's wording.
// ponytail: only the common JsonCpp messages are reproduced (empty text,
// a non-JSON first character, a scalar root); other syntax errors carry
// serde_json's message, and duplicate keys are not detected.
fn parse(s: &str) -> Result<J, String> {
    let Some((line, col, first)) = s.lines().enumerate().find_map(|(l, t)| {
        t.char_indices()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| (l + 1, t[..i].chars().count() + 1, c))
    }) else {
        return Err(format!(
            "* Line 1, Column 1\r  {VALUE_EXPECTED}\r* Line 1, Column 1\r  {NOT_A_DOCUMENT}"
        ));
    };
    match serde_json::from_str::<J>(s) {
        Ok(j @ (J::Object(_) | J::Array(_))) => Ok(j),
        Ok(_) => Err(format!("* Line {line}, Column {col}\r  {NOT_A_DOCUMENT}")),
        Err(_) if !"{[\"-0123456789tfn".contains(first) => Err(format!(
            "* Line {line}, Column {col}\r  {VALUE_EXPECTED}\r* Line {line}, Column {}\r  Extra non-whitespace after JSON value.",
            col + 1
        )),
        Err(e) => {
            // serde_json's messages in JsonCpp's words; at end of input
            // JsonCpp points one column further.
            let m = e.to_string();
            let (msg, past) = if m.starts_with("trailing characters") {
                ("Extra non-whitespace after JSON value.", 0)
            } else if m.starts_with("expected value") {
                (VALUE_EXPECTED, 0)
            } else if m.starts_with("EOF while parsing an object") {
                ("Missing ',' or '}' in object declaration", 1)
            } else if m.starts_with("EOF while parsing a list") {
                ("Missing ',' or ']' in array declaration", 1)
            } else if m.starts_with("key must be a string") {
                ("Missing '}' or object member name", 0)
            } else {
                return Err(format!("* Line {}, Column {}\r  {m}", e.line(), e.column()));
            };
            Err(format!(
                "* Line {}, Column {}\r  {msg}",
                e.line(),
                e.column() + past
            ))
        }
    }
}

/// Any JSON value (JSONRaw input).
fn parse_value(s: &str) -> Option<J> {
    serde_json::from_str(s).ok()
}

#[derive(Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
    Last,
    Append,
}

/// `a.b[2]['c.d'][:][+]`. Errors use JsonCpp's path message.
fn parse_path(p: &str) -> Result<Vec<Seg>, String> {
    parse_path_at(p).map_err(|at| format!("* Offset {at} in \"{p}\"\r  JSON path syntax error."))
}

fn parse_path_at(p: &str) -> Result<Vec<Seg>, usize> {
    let c: Vec<char> = p.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '.' => i += 1,
            '[' => {
                let close = if matches!(c.get(i + 1), Some('\'' | '"')) {
                    let q = c[i + 1];
                    let end = (i + 2..c.len()).find(|&j| c[j] == q).ok_or(i + 1)?;
                    out.push(Seg::Key(c[i + 2..end].iter().collect()));
                    end + 1
                } else {
                    let end = (i..c.len()).find(|&j| c[j] == ']').ok_or(i + 1)?;
                    let inner: String = c[i + 1..end].iter().collect();
                    out.push(match inner.trim() {
                        ":" => Seg::Last,
                        "+" => Seg::Append,
                        n => Seg::Index(n.parse().map_err(|_| i + 1)?),
                    });
                    end
                };
                if c.get(close) != Some(&']') {
                    return Err(close);
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
            _ => Err("in Json::Value::operator[](ArrayIndex): requires arrayValue".into()),
        },
        J::Array(a) => {
            let i = match seg {
                Seg::Append => a.len(),
                Seg::Last => a.len().saturating_sub(1),
                Seg::Index(i) => *i,
                Seg::Key(k) => k.parse().map_err(|_| {
                    "in Json::Value::resolveReference(key, end): requires objectValue".to_string()
                })?,
            };
            if a.len() <= i {
                a.resize(i + 1, J::Null);
            }
            set(&mut a[i], rest, value)
        }
        _ => Err(match seg {
            Seg::Key(_) => "in Json::Value::resolveReference(key, end): requires objectValue",
            _ => "in Json::Value::operator[](ArrayIndex): requires arrayValue",
        }
        .into()),
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

/// JsonCpp's number text: integers as written, other numbers as doubles
/// with 15 significant digits (`%.15g`), ".0" added when that looks whole
/// (1e3 -> 1000.0, 12345678901234567890123 -> 1.23456789012346e+22).
fn number_text(n: &Number) -> String {
    let s = n.to_string();
    if !s.contains(['.', 'e', 'E']) {
        if let Ok(i) = s.parse::<i64>() {
            return i.to_string();
        }
        if let Ok(u) = s.parse::<u64>() {
            return u.to_string();
        }
    }
    let g = g15(s.parse::<f64>().unwrap_or(0.0));
    if g.contains(['.', 'e', 'n', 'i']) {
        g
    } else {
        format!("{g}.0")
    }
}

/// C's `%.15g`, except that small numbers stay in plain notation
/// (FileMaker writes 10^-5 as 0.00001).
fn g15(f: f64) -> String {
    let g = g15_c(f);
    if let Some((_, e)) = g.split_once("e-") {
        let places = 14 + e.parse::<usize>().unwrap_or(0);
        let p = format!("{f:.places$}");
        return p.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    g
}

fn g15_c(f: f64) -> String {
    if f == 0.0 {
        return "0".into();
    }
    let e = format!("{f:.14e}");
    // inf and NaN have no exponent: return them as Rust prints them, which
    // `g15`'s callers already expect.
    let Some((m, Ok(exp))) = e.split_once('e').map(|(m, x)| (m, x.parse::<i32>())) else {
        return e;
    };
    let trim = |s: &str| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    };
    if !(-4..15).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(m), exp.abs())
    } else {
        trim(&format!("{f:.*}", (14 - exp).max(0) as usize))
    }
}

/// A calculation number as JSON: whole numbers that fit 64 bits exactly,
/// others through a double.
fn json_number(n: &num::Num) -> J {
    let n = num::shown(n);
    let text = if n.is_integer() && n.to_string().parse::<i64>().is_ok() {
        num::plain(&n)
    } else {
        g15(num::to_f64(&n))
    };
    Number::from_str(&text).map(J::Number).unwrap_or(J::Null)
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
    write_compact(j, &mut out, ":");
    out
}

/// Compact, but with " : " after keys (GetFieldsOnLayout's layout).
pub(crate) fn spaced(j: &J) -> String {
    let mut out = String::new();
    write_compact(j, &mut out, " : ");
    out
}

fn write_compact(j: &J, out: &mut String, colon: &str) {
    match j {
        J::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact(v, out, colon);
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
                out.push_str(colon);
                write_compact(v, out, colon);
            }
            out.push('}');
        }
        // Arrays and objects are matched above, so `scalar` always answers.
        s => out.push_str(&scalar(s).unwrap_or_default()),
    }
}

/// JsonCpp's StyledStreamWriter layout with tab indentation and CR line
/// ends (checked against FileMaker).
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
            s => self.out.push_str(&scalar(s).unwrap_or_default()),
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

/// The FileMaker value of an element: strings unquoted (line feeds become
/// returns), numbers as JsonCpp writes them, booleans as 1/0, null as
/// empty, containers as compact JSON.
fn element(j: &J) -> Value {
    match j {
        J::Null => Value::empty(),
        J::Bool(b) => Value::bool(*b),
        J::Number(n) => Value::Text(number_text(n)),
        J::String(s) => Value::text(s.replace("\r\n", "\r").replace('\n', "\r")),
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
            None => return err_text("Incorrect key, index, or path"),
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
        // null values are left out.
        let text = |c: &J| (!c.is_null()).then(|| element(c).to_text());
        let vals: Vec<String> = match j {
            Some(J::Object(m)) => m.values().filter_map(text).collect(),
            Some(J::Array(v)) => v.iter().filter_map(text).collect(),
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
        NUMBER => json_number(&v.num_or_zero()),
        BOOLEAN => J::Bool(match v.to_text().trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => v.truth(),
        }),
        NULL => J::Null,
        // Anything that is not an object (array) becomes an empty one.
        OBJECT => match parse_value(&v.to_text()) {
            Some(j @ J::Object(_)) => j,
            _ => J::Object(Map::new()),
        },
        ARRAY => match parse_value(&v.to_text()) {
            Some(j @ J::Array(_)) => j,
            _ => J::Array(vec![]),
        },
        // JSONRaw (and unknown types): JSON as is, anything else as a string.
        _ => {
            let s = v.to_text();
            parse_value(&s).unwrap_or(J::String(s))
        }
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

// JSONValueExists & co. (undocumented FileMaker 26 functions) match values
// by their FileMaker text, case-insensitively, and only inside arrays
// (oracle: ["1"] contains 1, {"a":1} contains nothing).

/// Matching key of a JSON value: its FileMaker text, case-insensitive, so
/// the number 1 matches "1" and "a" matches "A".
fn key(j: &J) -> String {
    element(j).to_text().to_lowercase()
}

/// The values of the array at the path (objects are not searched).
fn members(j: Option<&J>) -> Vec<String> {
    match j {
        Some(J::Array(a)) => a.iter().map(key).collect(),
        _ => vec![],
    }
}

fn count_of(a: &[Value]) -> R {
    let want = a[2].to_text().to_lowercase();
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
    match parse_value(&v.to_text()) {
        Some(J::Array(a)) => Ok(a.iter().map(key).collect()),
        Some(j) => Ok(vec![key(&j)]),
        None => parse(&v.to_text()).map(|j| vec![key(&j)]),
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
    if let J::Array(v) = node {
        v.retain(|x| keep(&key(x)));
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
    // A trailing separator makes a last, empty item ("1¶x¶" -> [1,0,0]).
    let items: Vec<&str> = if list.is_empty() {
        vec![]
    } else if sep.is_empty() || sep == "\r" {
        list.split(['\r', '\n']).collect()
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
