//! Calculation compiler: source text → token stream (key 5, keeps the
//! formatting) and compiled form (key 6, what FileMaker evaluates). Without
//! key 6 FileMaker evaluates a calculation to "?" (FORMAT.md §11).
//!
//! The compiled form is prefix notation without punctuation or trivia:
//! an operator or function token, then its operands. Functions with a
//! variable number of arguments write a u16 count and their arguments in
//! reverse order.

use crate::calc_tables::{EXTERNAL, FUNCTIONS, FUNCTIONS2, GET_FLAGS};
use crate::save::{encode_text, path_elem};

/// Names a calculation can refer to.
pub trait Resolve {
    /// Table occurrences: (id, name).
    fn occurrences(&self) -> Vec<(u32, String)>;
    /// Fields (id, name) of the base table behind occurrence `to`.
    fn fields(&self, to: u32) -> Vec<(u32, String)>;
    /// Custom functions: (id, name).
    fn custom_functions(&self) -> Vec<(u32, String)> {
        Vec::new()
    }
}

/// Keys 5 and 6 of a calculation node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub tokens: Vec<u8>,
    pub compiled: Vec<u8>,
    /// Fields referred to: (table occurrence, field id).
    pub fields: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Character offset in the source.
    pub at: usize,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at character {})", self.message, self.at)
    }
}

impl std::error::Error for CompileError {}

type Res<T> = Result<T, CompileError>;

fn err<T>(at: usize, message: impl Into<String>) -> Res<T> {
    Err(CompileError {
        at,
        message: message.into(),
    })
}

/// Compiles `src` for a calculation evaluated from occurrence `context`
/// (0 for scripts and layouts: every field is then written `TO::Field`).
pub fn compile(src: &str, context: u32, names: &dyn Resolve) -> Result<Compiled, CompileError> {
    compile_with(src, context, &[], names)
}

/// Compiles the calculation of a custom function with parameters
/// `params`: a parameter is written like a Let variable (`0B` name in key
/// 5, `18 slot` in key 6), numbered from 0 in order. No context: fields
/// are written `TO::Field`.
pub fn compile_function(
    src: &str,
    params: &[String],
    names: &dyn Resolve,
) -> Result<Compiled, CompileError> {
    compile_with(src, 0, params, names)
}

fn compile_with(
    src: &str,
    context: u32,
    params: &[String],
    names: &dyn Resolve,
) -> Result<Compiled, CompileError> {
    let (compiled, p) = parse(lex(src, context, names)?, params)?;
    let tokens = p.toks.iter().flat_map(|t| t.k5.iter().copied()).collect();
    let mut fields: Vec<(u32, u32)> = p
        .toks
        .iter()
        .filter_map(|t| match t.kind {
            Kind::Field { to, field, .. } => Some((to, field)),
            _ => None,
        })
        .collect();
    fields.sort_unstable();
    fields.dedup();
    Ok(Compiled {
        tokens,
        compiled,
        fields,
    })
}

/// Key 6 of a stored token stream (key 5). Scripts keep only key 5 in
/// memory ([`crate::scripts::Calc`]); the writer derives key 6 with this.
pub fn compiled(tokens: &[u8]) -> Result<Vec<u8>, CompileError> {
    Ok(parse(scan(tokens)?, &[])?.0)
}

/// Every function name the compiler knows, and every `Get` flag, sorted
/// (for a calculation editor's function list).
pub fn function_names() -> (Vec<&'static str>, Vec<&'static str>) {
    let mut f: Vec<&str> = FUNCTIONS
        .iter()
        .map(|(_, n)| *n)
        .chain(FUNCTIONS2.iter().map(|(_, _, n)| *n))
        .chain(EXTERNAL.iter().map(|(_, _, n)| *n))
        .filter(|n| *n != "Get")
        .collect();
    f.sort_unstable_by_key(|n| n.to_lowercase());
    f.dedup();
    let mut g: Vec<&str> = GET_FLAGS.iter().map(|(_, n)| *n).collect();
    g.sort_unstable_by_key(|n| n.to_lowercase());
    (f, g)
}

fn parse(toks: Vec<Tok>, params: &[String]) -> Res<(Vec<u8>, Parser)> {
    // A custom function's parameters are the outermost variables.
    let frames = if params.is_empty() {
        Vec::new()
    } else {
        vec![Frame {
            count: params.len() as u16,
            names: params
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i as u16))
                .collect(),
        }]
    };
    let mut p = Parser {
        toks,
        pos: 0,
        frames,
        // The whole calculation is in result position (FORMAT.md §16).
        tail: true,
    };
    p.skip();
    if p.pos >= p.toks.len() {
        // Only comments: FileMaker stores key 6 = 00 (as for text it
        // cannot parse, kept as one comment).
        let comment = p
            .toks
            .iter()
            .any(|t| matches!(t.k5.first(), Some(0x0D | 0x0E)));
        return if comment {
            Ok((vec![0], p))
        } else {
            err(0, "empty calculation")
        };
    }
    let c = p.expr(0)?;
    if p.pos < p.toks.len() {
        return err(p.toks[p.pos].at, "unexpected token");
    }
    Ok((c, p))
}

/// Tokens of a stored key-5 stream, as [`lex`] makes them from text
/// (`at` is a byte offset). Names stand for their token byte.
fn scan(b: &[u8]) -> Res<Vec<Tok>> {
    use crate::calc::{run, run_len};
    let mut out = Vec::new();
    let mut i = 0;
    let bad = |i: usize| CompileError {
        at: i,
        message: format!("unknown token {:02X?}", b.get(i)),
    };
    let ident = |s: &str| (Kind::Ident, s.to_string());
    while i < b.len() {
        let t = b[i];
        let run_at = |j: usize| -> Res<(String, usize)> {
            let (len, l) = run_len(b, j).ok_or_else(|| bad(i))?;
            let (s, n) = run(b, j + l, len).ok_or_else(|| bad(i))?;
            Ok((s, l + n + 1 + j - i))
        };
        let ((kind, text), len) = match t {
            0x0C..=0x0E => {
                let (s, n) = run_at(i + 2)?;
                ((Kind::Trivia, s), n)
            }
            0x13 => {
                let (s, n) = run_at(i + 1)?;
                ((Kind::Text, s), n)
            }
            // Let variable name: 0B len name 00 + 1 byte.
            0x0B => {
                let (s, n) = run_at(i + 1)?;
                (ident(&s), n + 1)
            }
            0x1D => {
                let lib = b.get(i + 4..i + 8).ok_or_else(|| bad(i))?;
                let idx = b.get(i + 8..i + 10).ok_or_else(|| bad(i))?;
                let idx = u16::from_be_bytes([idx[0], idx[1]]);
                let (_, _, n) = EXTERNAL
                    .iter()
                    .find(|(l, x, _)| l.as_slice() == lib && *x == idx)
                    .ok_or_else(|| bad(i))?;
                (ident(n), 10)
            }
            0x1B => {
                let (id, n) = b
                    .get(i + 1..)
                    .and_then(crate::calc::custom_ref)
                    .ok_or_else(|| bad(i))?;
                ((Kind::Custom(id), String::new()), 1 + n)
            }
            0x1A => {
                let units = *b.get(i + 1).ok_or_else(|| bad(i))? as usize;
                let n = (units..=units * 3 + 3)
                    .find(|n| {
                        b.get(i + 2..i + 2 + n)
                            .is_some_and(|s| crate::text(s).encode_utf16().count() == units)
                    })
                    .ok_or_else(|| bad(i))?;
                ((Kind::Var, String::new()), 2 + n)
            }
            0x10 => {
                let n = b
                    .get(i..)
                    .and_then(crate::calc::number_len)
                    .ok_or_else(|| bad(i))?;
                let c = b.get(i..i + n).ok_or_else(|| bad(i))?;
                ((Kind::Num(c.to_vec()), String::new()), n + 2)
            }
            0x16 => {
                let to = b
                    .get(i + 3..i + 7)
                    .and_then(crate::graph::to_ref)
                    .ok_or_else(|| bad(i))?;
                let n = *b.get(i + 7).ok_or_else(|| bad(i))? as usize;
                let field = b
                    .get(i + 9..i + 8 + n)
                    .and_then(crate::path_int)
                    .ok_or_else(|| bad(i))?;
                let qualified = b.get(i + 1).is_some_and(|f| f & 4 != 0);
                (
                    (
                        Kind::Field {
                            to,
                            field,
                            qualified,
                        },
                        String::new(),
                    ),
                    10 + n,
                )
            }
            0x9C => {
                let f = *b.get(i + 1).ok_or_else(|| bad(i))?;
                let (_, n) = GET_FLAGS
                    .iter()
                    .find(|(x, _)| *x == f)
                    .ok_or_else(|| bad(i))?;
                (ident(n), 2)
            }
            0xF8..=0xFE => {
                let x = *b.get(i + 1).ok_or_else(|| bad(i))?;
                let (_, _, n) = FUNCTIONS2
                    .iter()
                    .find(|(p, y, _)| *p == t && *y == x)
                    .ok_or_else(|| bad(i))?;
                (ident(n), 2)
            }
            // Unary minus may be stored as 2C.
            0x2C => ((Kind::Punct("-"), "-".into()), 1),
            _ => match PUNCT.iter().find(|(_, x)| *x == t) {
                Some((p, _)) => ((Kind::Punct(p), p.to_string()), 1),
                None => {
                    let n = crate::calc_tables::OPS
                        .iter()
                        .chain(FUNCTIONS)
                        .find(|(x, _)| *x == t)
                        .map(|(_, n)| *n)
                        .ok_or_else(|| bad(i))?;
                    (ident(n), 1)
                }
            },
        };
        let raw = b.get(i..i + len).ok_or_else(|| bad(i))?.to_vec();
        let k5 = match kind {
            Kind::Punct(p) => PUNCT
                .iter()
                .find(|(q, _)| *q == p)
                .map_or(raw, |(_, x)| vec![*x]),
            _ => raw,
        };
        out.push(Tok {
            kind,
            at: i,
            text,
            k5,
        });
        i += len;
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// Whitespace or comment: key 5 only.
    Trivia,
    /// Number literal: its compiled bytes.
    Num(Vec<u8>),
    Text,
    Field {
        to: u32,
        field: u32,
        qualified: bool,
    },
    /// `$local` / `$$global`.
    Var,
    Ident,
    /// Custom function call (its id).
    Custom(u32),
    Punct(&'static str),
}

#[derive(Debug, Clone)]
struct Tok {
    kind: Kind,
    /// Character offset.
    at: usize,
    text: String,
    /// Key 5 bytes (set by the parser for identifiers).
    k5: Vec<u8>,
}

/// `len` of a text run in a token: UTF-16 units, one byte below 0x80,
/// else two bytes with the top bit set.
fn run_len(s: &str, at: usize) -> Res<Vec<u8>> {
    let n = s.encode_utf16().count();
    match n {
        0..=0x7F => Ok(vec![n as u8]),
        0x80..=0x7FFF => Ok(vec![0x80 | (n >> 8) as u8, n as u8]),
        _ => err(at, "text too long"),
    }
}

/// `13 len text 00`, optionally after a trivia opcode.
fn text_run(prefix: Option<u8>, s: &str, at: usize) -> Res<Vec<u8>> {
    let mut out: Vec<u8> = prefix.into_iter().collect();
    out.push(0x13);
    out.extend(run_len(s, at)?);
    out.extend(encode_text(s));
    out.push(0);
    Ok(out)
}

/// Field reference `16 flags 04 TO len 01 field 00 01` (flags 04 when written `TO::Field`).
fn field_ref(op: u8, to: u32, field: u32, qualified: bool) -> Vec<u8> {
    let t = to.saturating_sub(128) as u16;
    let mut out = vec![op, if qualified { 4 } else { 0 }, 4, 3, 0xD0];
    out.extend(t.to_be_bytes());
    let f = path_elem(field);
    out.push(1 + f.len() as u8);
    out.push(1);
    out.extend(f);
    out.extend([0, 1]);
    out
}

/// Number literal: `10 02`, u16 number of integer groups, u16 number of
/// decimals (16, or more when the literal has more), then the integer and
/// the decimals in u32 groups of nine digits. Key 5 adds `00 20` (`00 40`
/// when written with an exponent). FORMAT.md §11.
fn number(lit: &str, at: usize) -> Res<(Vec<u8>, Vec<u8>)> {
    let lower = lit.to_ascii_lowercase();
    let (mant, exp) = match lower.split_once('e') {
        Some((m, e)) => (m, Some(e.parse::<i32>().map_err(|_| bad_number(at))?)),
        None => (lower.as_str(), None),
    };
    let (ip, fp) = mant.split_once('.').unwrap_or((mant, ""));
    let mut digits: Vec<u8> = ip.bytes().chain(fp.bytes()).map(|b| b - b'0').collect();
    // Position of the decimal point within `digits`.
    let mut point = ip.len() as i64 + exp.unwrap_or(0) as i64;
    if point < 0 {
        let pad = (-point) as usize;
        digits.splice(0..0, std::iter::repeat_n(0, pad));
        point = 0;
    }
    let point = point as usize;
    if digits.len() < point {
        digits.resize(point, 0);
    }
    let (int, frac) = digits.split_at(point);
    let int = &int[int.iter().take_while(|d| **d == 0).count()..];
    let frac = &frac[..frac.len() - frac.iter().rev().take_while(|d| **d == 0).count()];
    // Both parts are padded to whole groups of nine digits.
    let groups_of = |d: &[u8]| -> Vec<u8> {
        d.chunks(9)
            .flat_map(|g| g.iter().fold(0u32, |a, x| a * 10 + *x as u32).to_be_bytes())
            .collect()
    };
    // Integer groups are right-aligned: pad on the left to a multiple of 9.
    let n = int.len().div_ceil(9).max(1);
    let mut padded = vec![0; n * 9 - int.len()];
    padded.extend_from_slice(int);
    let l = frac.len().max(16);
    let mut decimals = frac.to_vec();
    // Room for two guard digits: 16 decimals take two groups, 17 take three.
    decimals.resize((l + 2).div_ceil(9) * 9, 0);
    if n > u16::MAX as usize || l > u16::MAX as usize {
        return err(at, format!("number literal {lit} is out of range"));
    }
    let mut c = vec![0x10, 2];
    c.extend((n as u16).to_be_bytes());
    c.extend((l as u16).to_be_bytes());
    c.extend(groups_of(&padded));
    c.extend(groups_of(&decimals));
    let mut k5 = c.clone();
    k5.extend([0, if exp.is_some() { 0x40 } else { 0x20 }]);
    Ok((c, k5))
}

fn bad_number(at: usize) -> CompileError {
    CompileError {
        at,
        message: "bad number".into(),
    }
}

const PUNCT: &[(&str, u8)] = &[
    ("<>", 0x45),
    ("<=", 0x42),
    (">=", 0x47),
    (";", 0x03),
    ("(", 0x04),
    (")", 0x05),
    ("[", 0x06),
    ("]", 0x07),
    ("¶", 0x24),
    ("+", 0x25),
    ("-", 0x26),
    ("*", 0x27),
    ("/", 0x28),
    ("^", 0x29),
    ("<", 0x41),
    ("≤", 0x43),
    ("=", 0x44),
    ("≠", 0x46),
    ("≥", 0x48),
    (">", 0x49),
    ("&", 0x50),
];

/// Characters that end a name.
fn stop(c: char) -> bool {
    c.is_whitespace() || "\"()[];+-*/^&=<>≠≤≥¶:".contains(c)
}

fn starts_ci(src: &[char], at: usize, pat: &str) -> Option<usize> {
    let mut i = at;
    for p in pat.chars() {
        let c = *src.get(i)?;
        if c != p && !c.to_lowercase().eq(p.to_lowercase()) {
            return None;
        }
        i += 1;
    }
    Some(i)
}

fn lex(src: &str, context: u32, names: &dyn Resolve) -> Res<Vec<Tok>> {
    let s: Vec<char> = src.chars().collect();
    // Field names, longest first: `TO::Field` everywhere, `Field` in context.
    let mut fields: Vec<(String, u32, u32, bool)> = Vec::new();
    for (to, tname) in names.occurrences() {
        for (f, fname) in names.fields(to) {
            fields.push((format!("{tname}::{fname}"), to, f, true));
            if to == context {
                fields.push((fname, to, f, false));
            }
        }
    }
    fields.sort_by_key(|f| std::cmp::Reverse(f.0.chars().count()));
    let customs = names.custom_functions();
    let mut out = Vec::new();
    let mut i = 0;
    let text = |a: usize, b: usize| s[a..b].iter().collect::<String>();
    while i < s.len() {
        let at = i;
        let c = s[i];
        let next = s.get(i + 1).copied();
        // Trivia.
        if c.is_whitespace() {
            while i < s.len() && s[i].is_whitespace() {
                i += 1;
            }
            let t = text(at, i);
            out.push(Tok {
                kind: Kind::Trivia,
                at,
                k5: text_run(Some(0x0C), &t, at)?,
                text: t,
            });
            continue;
        }
        if c == '/' && next == Some('*') {
            let end = (i + 2..s.len().saturating_sub(1))
                .find(|&j| s[j] == '*' && s[j + 1] == '/')
                .ok_or_else(|| CompileError {
                    at,
                    message: "unterminated comment".into(),
                })?;
            let inner = text(i + 2, end);
            i = end + 2;
            out.push(Tok {
                kind: Kind::Trivia,
                at,
                k5: text_run(Some(0x0D), &inner, at)?,
                text: text(at, i),
            });
            continue;
        }
        if c == '/' && next == Some('/') {
            let mut j = i + 2;
            while j < s.len() && s[j] != '\r' && s[j] != '\n' {
                j += 1;
            }
            // The line break belongs to the comment.
            if j < s.len() {
                j += if s[j] == '\r' && s.get(j + 1) == Some(&'\n') {
                    2
                } else {
                    1
                };
            }
            let rest = text(i + 2, j);
            i = j;
            out.push(Tok {
                kind: Kind::Trivia,
                at,
                k5: text_run(Some(0x0E), &rest, at)?,
                text: text(at, i),
            });
            continue;
        }
        // Text literal.
        if c == '"' {
            let mut v = String::new();
            i += 1;
            loop {
                let Some(&ch) = s.get(i) else {
                    return err(at, "unterminated text");
                };
                i += 1;
                match ch {
                    '"' => break,
                    '\\' => match s.get(i) {
                        Some('"') | Some('\\') => {
                            v.push(s[i]);
                            i += 1;
                        }
                        Some('¶') => {
                            v.push('¶');
                            i += 1;
                        }
                        _ => v.push('\\'),
                    },
                    '¶' => v.push('\r'),
                    _ => v.push(ch),
                }
            }
            out.push(Tok {
                kind: Kind::Text,
                at,
                k5: text_run(None, &v, at)?,
                text: v,
            });
            continue;
        }
        // Number.
        if c.is_ascii_digit() || c == '.' && next.is_some_and(|d| d.is_ascii_digit()) {
            while i < s.len() && (s[i].is_ascii_digit() || s[i] == '.') {
                i += 1;
            }
            if i < s.len() && (s[i] == 'e' || s[i] == 'E') {
                let mut j = i + 1;
                if j < s.len() && (s[j] == '+' || s[j] == '-') {
                    j += 1;
                }
                if j < s.len() && s[j].is_ascii_digit() {
                    i = j;
                    while i < s.len() && s[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let lit = text(at, i);
            if lit.matches('.').count() > 1 {
                return err(at, "bad number");
            }
            let (c6, k5) = number(&lit, at)?;
            out.push(Tok {
                kind: Kind::Num(c6),
                at,
                k5,
                text: lit,
            });
            continue;
        }
        // Field names (they may hold spaces and operator characters).
        let field = fields.iter().find_map(|(name, to, f, q)| {
            let end = starts_ci(&s, i, name)?;
            let boundary = s.get(end).is_none_or(|c| stop(*c)) || name.ends_with(stop);
            boundary.then_some((end, *to, *f, *q))
        });
        if let Some((end, to, f, q)) = field {
            i = end;
            out.push(Tok {
                kind: Kind::Field {
                    to,
                    field: f,
                    qualified: q,
                },
                at,
                k5: field_ref(0x16, to, f, q),
                text: text(at, end),
            });
            continue;
        }
        // Operators and punctuation.
        if let Some((p, b)) = PUNCT.iter().find(|(p, _)| starts_ci(&s, i, p).is_some()) {
            i += p.chars().count();
            out.push(Tok {
                kind: Kind::Punct(p),
                at,
                k5: vec![*b],
                text: p.to_string(),
            });
            continue;
        }
        // Words: function names, keywords, Let variables, $variables.
        while i < s.len() && !stop(s[i]) {
            i += 1;
        }
        if i == at {
            return err(at, format!("unexpected character {c:?}"));
        }
        let w = text(at, i);
        if w.starts_with('$') {
            let mut k5 = vec![0x1A];
            k5.extend(run_len(&w, at)?);
            k5.extend(encode_text(&w));
            out.push(Tok {
                kind: Kind::Var,
                at,
                k5,
                text: w,
            });
        } else if let Some((id, _)) = customs.iter().find(|(_, n)| n.eq_ignore_ascii_case(&w)) {
            out.push(Tok {
                kind: Kind::Custom(*id),
                at,
                k5: custom_token(*id),
                text: w,
            });
        } else {
            out.push(Tok {
                kind: Kind::Ident,
                at,
                k5: Vec::new(),
                text: w,
            });
        }
    }
    Ok(out)
}

/// Key 5 of a custom function call: `1B 14 len 01 id`.
fn custom_token(id: u32) -> Vec<u8> {
    let f = path_elem(id);
    let mut out = vec![0x1B, 0x14, 1 + f.len() as u8, 1];
    out.extend(f);
    out
}

/// Function token classes.
#[derive(Clone, Copy)]
enum Func {
    One(u8),
    Two(u8, u8),
    External(&'static [u8; 4], u16),
}

impl Func {
    fn bytes(self) -> Vec<u8> {
        match self {
            Func::One(t) => vec![t],
            Func::Two(p, x) => vec![p, x],
            Func::External(lib, i) => {
                let mut b = vec![0x1D, 0, 0, 0x15];
                b.extend(lib);
                b.extend(i.to_be_bytes());
                b
            }
        }
    }
}

/// Token of a function name. A few names are both a function and a
/// constant (Extend, Lower: style and LookupNext constants): `call` picks.
fn lookup_func(name: &str, call: bool) -> Option<Func> {
    let eq = |n: &str| n.eq_ignore_ascii_case(name);
    let mut ones = FUNCTIONS
        .iter()
        .filter(|(_, n)| eq(n))
        .map(|(t, _)| Func::One(*t));
    let first = ones.next();
    if let Some(f) = first {
        return Some(match ones.next() {
            Some(c) if !call => c,
            _ => f,
        });
    }
    if let Some((p, x, _)) = FUNCTIONS2.iter().find(|(_, _, n)| eq(n)) {
        return Some(Func::Two(*p, *x));
    }
    EXTERNAL
        .iter()
        .find(|(_, _, n)| eq(n))
        .map(|(l, i, _)| Func::External(l, *i))
}

/// Functions written with a u16 argument count and their arguments in
/// reverse order (those with optional or repeated arguments): Factorial,
/// Substitute, the aggregates, TextFont, the Text…Remove functions, Lookup,
/// LookupNext, every `F9 xx` and external (`1D`) function.
fn variadic(f: Func) -> bool {
    match f {
        Func::One(t) => matches!(
            t,
            0x3B | 0x63 | 0x7F..=0x85 | 0x87 | 0xA0..=0xA2 | 0xA8 | 0xA9 | 0xBA | 0xE0 | 0xE1
        ),
        Func::Two(p, _) => p == 0xF9,
        Func::External(..) => true,
    }
}

/// Functions that take a field itself (all its values or repetitions):
/// a field argument is written with op 0x17 in the compiled form.
fn takes_field(f: Func) -> bool {
    matches!(f, Func::One(0x7F..=0x8B | 0xA0 | 0xA1 | 0xE0 | 0xE1))
}

/// Names that may be written without parentheses.
fn constant(f: Func) -> bool {
    match f {
        Func::One(t) => matches!(t, 0x31 | 0x97 | 0xAA..=0xC8),
        Func::Two(p, _) => p == 0xFA,
        Func::External(_, i) => matches!(i, 0x01 | 0x13),
    }
}

/// One compiled argument; `field` is the 0x17 form of a bare field,
/// `list` the items of a `[ … ]` list.
struct Arg {
    code: Vec<u8>,
    field: Option<Vec<u8>>,
    list: Option<Vec<Vec<u8>>>,
}

/// Variables of one Let or While: bindings so far, named ones with their slot.
struct Frame {
    count: u16,
    names: Vec<(String, u16)>,
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    frames: Vec<Frame>,
    /// The expression parsed next gives the calculation's result (a custom
    /// function called there is compiled `1C`, elsewhere `1B`). Taken
    /// (reset) by whatever parses next.
    tail: bool,
}

fn u16b(n: usize) -> [u8; 2] {
    (n as u16).to_be_bytes()
}

impl Parser {
    fn skip(&mut self) {
        while self.pos < self.toks.len() && self.toks[self.pos].kind == Kind::Trivia {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn at(&self) -> usize {
        self.peek()
            .map_or_else(|| self.toks.last().map_or(0, |t| t.at), |t| t.at)
    }

    fn bump(&mut self) -> usize {
        let i = self.pos;
        self.pos += 1;
        self.skip();
        i
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok { kind: Kind::Punct(q), .. }) if *q == p)
    }

    fn expect(&mut self, p: &str) -> Res<()> {
        if self.is_punct(p) {
            self.bump();
            Ok(())
        } else {
            err(self.at(), format!("expected {p:?}"))
        }
    }

    fn keyword(&self) -> Option<(&'static str, u8)> {
        let t = self.peek()?;
        if t.kind != Kind::Ident {
            return None;
        }
        [("and", 0x1F), ("or", 0x20), ("xor", 0x21), ("not", 0x1E)]
            .into_iter()
            .find(|(k, _)| t.text.eq_ignore_ascii_case(k))
    }

    /// Binary operator at the cursor: (compiled byte, level).
    fn binop(&self) -> Option<(u8, u8)> {
        if let Some((k, b)) = self.keyword() {
            return match k {
                "and" => Some((b, 2)),
                "or" | "xor" => Some((b, 1)),
                _ => None,
            };
        }
        let Some(Tok {
            kind: Kind::Punct(p),
            k5,
            ..
        }) = self.peek()
        else {
            return None;
        };
        let level = match *p {
            "=" | "<>" | "≠" | "<" | "<=" | "≤" | ">" | ">=" | "≥" => 3,
            "&" => 4,
            "+" | "-" => 5,
            "*" | "/" => 6,
            "^" => 7,
            _ => return None,
        };
        Some((k5[0], level))
    }

    /// Precedence climbing; all binary operators are left-associative.
    fn expr(&mut self, min: u8) -> Res<Vec<u8>> {
        let tail = std::mem::take(&mut self.tail);
        let start = self.pos;
        self.tail = tail;
        let mut lhs = self.unary()?;
        // An operand of an operator is not the result: parse it again.
        if tail && self.binop().is_some_and(|(_, l)| l >= min) {
            self.pos = start;
            lhs = self.unary()?;
        }
        while let Some((op, level)) = self.binop().filter(|(_, l)| *l >= min) {
            let i = self.bump();
            if let Some((_, b)) = [("and", 0x1F), ("or", 0x20), ("xor", 0x21)]
                .into_iter()
                .find(|(k, _)| self.toks[i].text.eq_ignore_ascii_case(k))
            {
                self.toks[i].k5 = vec![b];
            }
            let rhs = self.expr(level + 1)?;
            lhs = [vec![op], lhs, rhs].concat();
        }
        Ok(lhs)
    }

    /// Prefix operators bind tighter than `^`.
    fn unary(&mut self) -> Res<Vec<u8>> {
        let tail = std::mem::take(&mut self.tail);
        let op = if self.is_punct("-") {
            Some(0x2C)
        } else if self.is_punct("+") {
            Some(0x68)
        } else if self.keyword().is_some_and(|k| k.0 == "not") {
            let i = self.pos;
            self.toks[i].k5 = vec![0x1E];
            Some(0x1E)
        } else {
            None
        };
        match op {
            Some(op) => {
                self.bump();
                Ok([vec![op], self.unary()?].concat())
            }
            None => {
                self.tail = tail;
                self.primary()
            }
        }
    }

    /// Parses an expression that is (`tail`) or is not the result.
    fn expr_tail(&mut self, tail: bool) -> Res<Vec<u8>> {
        self.tail = tail;
        self.expr(0)
    }

    fn primary(&mut self) -> Res<Vec<u8>> {
        let tail = std::mem::take(&mut self.tail);
        let at = self.at();
        let Some(t) = self.peek().cloned() else {
            return err(at, "unexpected end of calculation");
        };
        match &t.kind {
            Kind::Num(c) => {
                self.bump();
                Ok(c.clone())
            }
            Kind::Text => {
                self.bump();
                Ok(t.k5.clone())
            }
            Kind::Punct("¶") => {
                self.bump();
                Ok(vec![0x24])
            }
            Kind::Punct("(") => {
                self.bump();
                let e = self.expr_tail(tail)?;
                self.expect(")")?;
                Ok(e)
            }
            // A Let variable or parameter may shadow a custom function.
            Kind::Custom(_) if self.slot_of(&t.text).is_some() => {
                let i = self.pos;
                self.toks[i].kind = Kind::Ident;
                self.ident(tail)
            }
            Kind::Custom(id) => {
                self.bump();
                let args = if self.is_punct("(") {
                    self.args()?
                } else {
                    Vec::new()
                };
                let f = path_elem(*id);
                let mut c = vec![if tail { 0x1C } else { 0x1B }];
                c.extend(u16b(args.len()));
                c.extend([0x14, 1 + f.len() as u8, 1]);
                c.extend(f);
                c.extend(args.iter().rev().flat_map(|a| a.code.iter().copied()));
                Ok(c)
            }
            Kind::Var => {
                self.bump();
                match self.repetition()? {
                    Some(r) => Ok([vec![0xE4], r, t.k5].concat()),
                    None => Ok(t.k5),
                }
            }
            // A Let variable may shadow a field name.
            Kind::Field {
                qualified: false, ..
            } if self.slot_of(&t.text).is_some() => {
                let i = self.pos;
                self.toks[i].kind = Kind::Ident;
                self.ident(tail)
            }
            Kind::Field {
                to,
                field,
                qualified,
            } => {
                self.bump();
                match self.repetition()? {
                    // `Field[n]` compiles as GetRepetition ( Field ; n ).
                    Some(r) => {
                        Ok([vec![0x89], field_ref(0x17, *to, *field, *qualified), r].concat())
                    }
                    None => Ok(t.k5),
                }
            }
            Kind::Ident => self.ident(tail),
            _ => err(at, format!("unexpected {:?}", t.text)),
        }
    }

    /// `[ expr ]` after a variable or field.
    fn repetition(&mut self) -> Res<Option<Vec<u8>>> {
        if !self.is_punct("[") {
            return Ok(None);
        }
        self.bump();
        let r = self.expr(0)?;
        self.expect("]")?;
        Ok(Some(r))
    }

    fn slot_of(&self, name: &str) -> Option<u16> {
        self.frames.iter().rev().find_map(|f| {
            f.names
                .iter()
                .rev()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, s)| *s)
        })
    }

    fn ident(&mut self, tail: bool) -> Res<Vec<u8>> {
        let i = self.pos;
        let at = self.toks[i].at;
        let name = self.toks[i].text.clone();
        if name.eq_ignore_ascii_case("true") || name.eq_ignore_ascii_case("false") {
            let b = if name.eq_ignore_ascii_case("true") {
                0x22
            } else {
                0x23
            };
            self.toks[i].k5 = vec![b];
            self.bump();
            return Ok(vec![b]);
        }
        if let Some(slot) = self.slot_of(&name) {
            self.toks[i].k5 = let_name(&name, at)?;
            self.bump();
            return Ok([vec![0x18], slot.to_be_bytes().to_vec()].concat());
        }
        self.bump();
        let Some(f) = lookup_func(&name, self.is_punct("(")) else {
            return err(at, format!("unknown name {name:?}"));
        };
        self.toks[i].k5 = f.bytes();
        match f {
            Func::One(0x9B) => {
                self.expect("(")?;
                let j = self.pos;
                let flag = self
                    .peek()
                    .filter(|t| t.kind == Kind::Ident)
                    .and_then(|t| {
                        // The old name of CurrentPrivilegeSetName (same token).
                        let name = if t.text.eq_ignore_ascii_case("PrivilegeSetName") {
                            "CurrentPrivilegeSetName"
                        } else {
                            &t.text
                        };
                        GET_FLAGS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name))
                    })
                    .map(|(b, _)| *b);
                let Some(flag) = flag else {
                    return err(self.at(), "unknown Get flag");
                };
                self.toks[j].k5 = vec![0x9C, flag];
                self.bump();
                self.expect(")")?;
                Ok(vec![0x9B, flag])
            }
            Func::One(0xDF) => self.let_(tail),
            Func::Two(0xF8, 0) => self.while_(tail),
            _ if !self.is_punct("(") => {
                if constant(f) {
                    Ok(f.bytes())
                } else {
                    err(at, format!("{name} needs parameters"))
                }
            }
            _ => {
                // If, Case and Choose pass the result position to their results.
                let results: fn(usize, bool) -> bool = match f {
                    Func::One(0x4B | 0x4D) => |i, _| i > 0,
                    Func::One(0x4C) => |i, last| i % 2 == 1 || last,
                    _ => |_, _| false,
                };
                let args = self.args_tail(tail, results)?;
                Ok(emit_call(f, args))
            }
        }
    }

    /// `( a ; b ; … )`; an argument may be a `[ … ]` list.
    fn args(&mut self) -> Res<Vec<Arg>> {
        self.args_tail(false, |_, _| false)
    }

    /// [`Self::args`]; argument `i` is in result position when `tail` and
    /// `result(i, is the last argument)`.
    fn args_tail(&mut self, tail: bool, result: fn(usize, bool) -> bool) -> Res<Vec<Arg>> {
        self.expect("(")?;
        let mut out = Vec::new();
        if self.is_punct(")") {
            self.bump();
            return Ok(out);
        }
        loop {
            if self.is_punct("[") {
                self.bump();
                let mut items = Vec::new();
                loop {
                    items.push(self.expr(0)?);
                    if self.is_punct(";") {
                        self.bump();
                    } else {
                        break;
                    }
                }
                self.expect("]")?;
                let mut code = vec![0x19];
                code.extend(u16b(items.len()));
                code.extend(items.iter().rev().flatten());
                out.push(Arg {
                    code,
                    field: None,
                    list: Some(items),
                });
            } else {
                let start = self.pos;
                let i = out.len();
                let mut code = self.expr_tail(tail && result(i, true))?;
                // Whether it was the last argument shows only now.
                let last = !self.is_punct(";");
                if tail && result(i, true) != result(i, last) {
                    self.pos = start;
                    code = self.expr_tail(result(i, last))?;
                }
                let single = self.toks[start..self.pos]
                    .iter()
                    .filter(|t| t.kind != Kind::Trivia)
                    .count()
                    == 1;
                let field = match &self.toks[start].kind {
                    Kind::Field {
                        to,
                        field,
                        qualified,
                    } if single => Some(field_ref(0x17, *to, *field, *qualified)),
                    _ => None,
                };
                out.push(Arg {
                    code,
                    field,
                    list: None,
                });
            }
            if self.is_punct(";") {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(")")?;
        Ok(out)
    }

    /// One binding `name = value` of a Let or While; returns its compiled form.
    fn binding(&mut self) -> Res<Vec<u8>> {
        let at = self.at();
        let Some(t) = self.peek().cloned() else {
            return err(at, "expected a variable");
        };
        let frame = self.frames.len() - 1;
        let slot: u16 = self.frames.iter().map(|f| f.count).sum();
        match t.kind {
            Kind::Var => {
                self.bump();
                let rep = self.repetition()?;
                self.expect("=")?;
                let value = self.expr(0)?;
                self.frames[frame].count += 1;
                let rep = match rep {
                    Some(r) => r,
                    None => number("1", at)?.0,
                };
                Ok([vec![0xE3], value, rep, t.k5].concat())
            }
            // A Let variable may shadow a field name.
            Kind::Ident
            | Kind::Field {
                qualified: false, ..
            } => {
                let i = self.pos;
                self.toks[i].kind = Kind::Ident;
                self.toks[i].k5 = let_name(&t.text, at)?;
                self.bump();
                self.expect("=")?;
                let value = self.expr(0)?;
                let f = &mut self.frames[frame];
                f.count += 1;
                let old = f
                    .names
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&t.text))
                    .map(|(_, s)| *s);
                Ok(match old {
                    // Reassignment: value, repetition 1, target slot.
                    Some(s) => [
                        vec![0xE3],
                        value,
                        number("1", at)?.0,
                        vec![0x18],
                        s.to_be_bytes().to_vec(),
                    ]
                    .concat(),
                    None => {
                        f.names.push((t.text.clone(), slot));
                        value
                    }
                })
            }
            _ => err(at, "expected a variable"),
        }
    }

    /// A single binding or a `[ … ]` list: u16 count, then the bindings.
    fn bindings(&mut self) -> Res<Vec<u8>> {
        let mut items = Vec::new();
        if self.is_punct("[") {
            self.bump();
            loop {
                items.push(self.binding()?);
                if self.is_punct(";") {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect("]")?;
        } else {
            items.push(self.binding()?);
        }
        Ok([u16b(items.len()).to_vec(), items.concat()].concat())
    }

    fn let_(&mut self, tail: bool) -> Res<Vec<u8>> {
        self.expect("(")?;
        self.frames.push(Frame {
            count: 0,
            names: Vec::new(),
        });
        let b = self.bindings()?;
        self.expect(";")?;
        let body = self.expr_tail(tail)?;
        self.expect(")")?;
        self.frames.pop();
        Ok([vec![0xDF], b, body].concat())
    }

    fn while_(&mut self, tail: bool) -> Res<Vec<u8>> {
        self.expect("(")?;
        self.frames.push(Frame {
            count: 0,
            names: Vec::new(),
        });
        let init = self.bindings()?;
        self.expect(";")?;
        let cond = self.expr(0)?;
        self.expect(";")?;
        let lp = self.bindings()?;
        self.expect(";")?;
        let result = self.expr_tail(tail)?;
        self.expect(")")?;
        self.frames.pop();
        Ok([vec![0xF8, 0], init, cond, lp, result].concat())
    }
}

/// Key 5 of a Let variable name: `0B len name 00 00`.
fn let_name(name: &str, at: usize) -> Res<Vec<u8>> {
    let mut v = vec![0x0B];
    v.extend(run_len(name, at)?);
    v.extend(encode_text(name));
    v.extend([0, 0]);
    Ok(v)
}

fn emit_call(f: Func, args: Vec<Arg>) -> Vec<u8> {
    // Evaluate: count, 00 00, then the expression and the trigger fields in
    // order (a list of fields is spread).
    if let Func::One(0xDC) = f {
        let codes: Vec<Vec<u8>> = args
            .into_iter()
            .flat_map(|a| a.list.unwrap_or_else(|| vec![a.code]))
            .collect();
        return [
            vec![0xDC],
            u16b(codes.len()).to_vec(),
            vec![0, 0],
            codes.concat(),
        ]
        .concat();
    }
    let fields = takes_field(f);
    let n = args.len();
    let mut codes: Vec<Vec<u8>> = args
        .into_iter()
        .map(|a| match a.field {
            Some(x) if fields => x,
            _ => a.code,
        })
        .collect();
    match f {
        // Case: number of test/result pairs; a default result gets the test 1.
        Func::One(0x4C) => {
            if n % 2 == 1 {
                let d = codes.pop().unwrap_or_default();
                codes.push(number("1", 0).map(|x| x.0).unwrap_or_default());
                codes.push(d);
            }
            [vec![0x4C], u16b(codes.len() / 2).to_vec(), codes.concat()].concat()
        }
        // If without an else result: 00 in its place.
        Func::One(0x4B) if n == 2 => [vec![0x4B], codes.concat(), vec![0]].concat(),
        // Choose: number of results, arguments in order.
        Func::One(0x4D) => [
            vec![0x4D],
            u16b(n.saturating_sub(1)).to_vec(),
            codes.concat(),
        ]
        .concat(),
        Func::One(0xDD) => [vec![0xDD, 0, 0], codes.concat()].concat(),
        // SetPrecision, SetRecursion: arguments in reverse order, no count.
        Func::One(0x3D | 0x40) => {
            codes.reverse();
            [f.bytes(), codes.concat()].concat()
        }
        Func::External(..) => {
            let mut b = f.bytes();
            b[1..3].copy_from_slice(&u16b(n));
            codes.reverse();
            [b, codes.concat()].concat()
        }
        _ if variadic(f) => {
            codes.reverse();
            [f.bytes(), u16b(n).to_vec(), codes.concat()].concat()
        }
        _ => [f.bytes(), codes.concat()].concat(),
    }
}
