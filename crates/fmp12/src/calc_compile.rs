//! Calculation compiler: source text → the two token streams FileMaker
//! stores next to a calculation (`docs/FORMAT.md` §11).
//!
//! * key 5, the *source* stream: one token per lexical item, whitespace
//!   and comments included (`0C 13 len text 00`), so it decompiles back to
//!   the text as typed ([`crate::calc::decompile`]);
//! * key 6, the *compiled* stream: the same atoms in prefix (Polish)
//!   order, without whitespace or parentheses. Operators come before their
//!   operands, a function before its arguments, numbers lose their two
//!   trailing flag bytes, `Get ( Flag )` becomes `9B flag`. Functions that
//!   take any number of arguments write a u16 count after their token, and
//!   aggregate functions (`Sum`, `List`…) mark field arguments `17`
//!   instead of `16`.
//!
//! [`compiled`] derives key 6 from key 5; it reproduces every pair stored
//! in the FileMaker starter templates (see the tests).
//!
//! Self-contained on purpose (an editing agent may write another compiler
//! for schema calculations; the two are to be merged).

use crate::calc_tables::{FUNCTIONS, FUNCTIONS2, GET_FLAGS, OPS};
use crate::save::{encode_text, path_elem};

/// Resolves field names while compiling.
pub trait Resolve {
    /// (table occurrence, field id) of `TO::Field` (`to` given) or of a
    /// field of the context occurrence (`to` = `None`).
    fn field(&self, to: Option<&str>, name: &str) -> Option<(u32, u32)>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Character offset in the source.
    pub at: usize,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at {})", self.message, self.at)
    }
}

impl std::error::Error for CompileError {}

/// Both streams of a compiled calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// Key 5: source tokens.
    pub tokens: Vec<u8>,
    /// Key 6: prefix form.
    pub compiled: Vec<u8>,
}

/// Compiles `src`. `context` is the calculation's table occurrence (0 for
/// scripts and layouts, whose field references always name their TO).
pub fn compile(src: &str, context: u32, r: &dyn Resolve) -> Result<Compiled, CompileError> {
    let tokens = source_tokens(src, context, r)?;
    let compiled = compiled(&tokens).map_err(|at| CompileError {
        at: 0,
        message: format!("syntax error near token {at}"),
    })?;
    Ok(Compiled { tokens, compiled })
}

fn err<T>(at: usize, message: impl Into<String>) -> Result<T, CompileError> {
    Err(CompileError {
        at,
        message: message.into(),
    })
}

/// `13 len text 00` (len in UTF-16 units).
fn text_token(op: u8, s: &str) -> Vec<u8> {
    let mut out = vec![op, s.encode_utf16().count() as u8];
    out.extend(encode_text(s));
    out.push(0);
    out
}

/// Text split into runs of at most 255 UTF-16 units.
fn runs(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut n = 0;
    for c in s.chars() {
        if n + c.len_utf16() > 255 {
            out.push(String::new());
            n = 0;
        }
        n += c.len_utf16();
        if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    out
}

/// Number literal: `10 02 00 01 00 10`, u32 integer, two u32 groups of
/// nine decimals, flags `00 20`.
fn number_token(s: &str) -> Option<Vec<u8>> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let int: u32 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let mut f = frac.to_string();
    if f.len() > 18 || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    while f.len() < 18 {
        f.push('0');
    }
    let a: u32 = f[..9].parse().ok()?;
    let b: u32 = f[9..].parse().ok()?;
    let mut out = vec![0x10, 0x02, 0x00, 0x01, 0x00, 0x10];
    out.extend(int.to_be_bytes());
    out.extend(a.to_be_bytes());
    out.extend(b.to_be_bytes());
    out.extend([0x00, 0x20]);
    Some(out)
}

/// `16 flags 04 03 D0 hi lo len 01 field u16 repetition`. Flags are 04 in
/// script and layout calculations (no context occurrence), 00 in fields.
fn field_token(context: u32, to: u32, field: u32) -> Vec<u8> {
    let t = to.saturating_sub(128) as u16;
    let f = path_elem(field);
    let mut out = vec![0x16, if to == context { 0 } else { 4 }, 4, 3, 0xD0];
    out.extend(t.to_be_bytes());
    out.push(1 + f.len() as u8);
    out.push(1);
    out.extend(f);
    out.extend([0, 1]);
    out
}

fn lookup_name(name: &str) -> Option<Vec<u8>> {
    if let Some((t, _)) = FUNCTIONS
        .iter()
        .chain(OPS.iter().filter(|(t, _)| (0x1E..=0x23).contains(t)))
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
    {
        return Some(vec![*t]);
    }
    FUNCTIONS2
        .iter()
        .find(|(_, _, n)| n.eq_ignore_ascii_case(name))
        .map(|(p, x, _)| vec![*p, *x])
}

/// Characters that end a name.
fn delimiter(c: char) -> bool {
    "()[];\"&+-*/^=<>≠≤≥¶\r\n\t$".contains(c)
}

/// Key 5 for `src`.
pub fn source_tokens(src: &str, context: u32, r: &dyn Resolve) -> Result<Vec<u8>, CompileError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // Whether the last significant token ends an operand (for binary minus).
    let mut operand = false;
    let mut after_get = false;
    let has_let = src.to_lowercase().contains("let");
    let mut let_vars: Vec<String> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().take(2).collect();
        // Whitespace and comments.
        if c.is_whitespace() || rest == "//" || rest == "/*" {
            let start = i;
            loop {
                let two: String = chars[i..].iter().take(2).collect();
                if i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                } else if two == "//" {
                    while i < chars.len() && chars[i] != '\r' && chars[i] != '\n' {
                        i += 1;
                    }
                } else if two == "/*" {
                    i += 2;
                    while i < chars.len() && !chars[i..].starts_with(&['*', '/']) {
                        i += 1;
                    }
                    if i >= chars.len() {
                        return err(start, "unterminated comment");
                    }
                    i += 2;
                } else {
                    break;
                }
            }
            let ws: String = chars[start..i].iter().collect();
            for run in runs(&ws) {
                out.push(0x0C);
                out.extend(text_token(0x13, &run));
            }
            continue;
        }
        if c == '"' {
            let start = i;
            i += 1;
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None => return err(start, "unterminated text"),
                    Some('"') => break,
                    Some('\\') if matches!(chars.get(i + 1), Some('"' | '\\')) => {
                        s.push(chars[i + 1]);
                        i += 2;
                    }
                    Some(x) => {
                        s.push(*x);
                        i += 1;
                    }
                }
            }
            i += 1;
            // ponytail: literals over 255 UTF-16 units are split into a
            // concatenation; FileMaker's own long-literal token is unknown.
            for (k, run) in runs(&s).iter().enumerate() {
                if k > 0 {
                    out.push(0x50);
                }
                out.extend(text_token(0x13, run));
            }
            operand = true;
            continue;
        }
        if c.is_ascii_digit() || c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            match number_token(&s) {
                Some(t) => out.extend(t),
                None => return err(start, format!("unsupported number {s}")),
            }
            operand = true;
            continue;
        }
        if c == '$' {
            let start = i;
            while i < chars.len() && chars[i] == '$' {
                i += 1;
            }
            while i < chars.len() && !delimiter(chars[i]) && !chars[i].is_whitespace() {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            out.push(0x1A);
            out.push(name.encode_utf16().count() as u8);
            out.extend(encode_text(&name));
            operand = true;
            continue;
        }
        let sym: Option<(usize, u8)> = match (c, chars.get(i + 1)) {
            ('<', Some('>')) => Some((2, 0x45)),
            ('<', Some('=')) => Some((2, 0x42)),
            ('>', Some('=')) => Some((2, 0x47)),
            ('-', _) if !operand => Some((1, 0x2C)),
            _ => OPS
                .iter()
                .find(|(t, s)| *t != 0x2C && s.chars().count() == 1 && s.starts_with(c))
                .map(|(t, _)| (1, *t)),
        };
        if let Some((n, t)) = sym {
            out.push(t);
            i += n;
            operand = matches!(t, 0x05 | 0x07);
            continue;
        }
        // A name: function, keyword, Get flag or field (names may hold spaces).
        let start = i;
        let mut end = i;
        while end < chars.len() && !delimiter(chars[end]) {
            end += 1;
        }
        let words: String = chars[start..end].iter().collect();
        // Candidate lengths: every prefix ending before a space, longest first.
        let mut cuts: Vec<usize> = words
            .char_indices()
            .filter(|(_, ch)| *ch == ' ')
            .map(|(k, _)| words[..k].chars().count())
            .collect();
        cuts.push(words.chars().count());
        cuts.retain(|k| *k > 0);
        cuts.reverse();
        let mut found = None;
        for k in cuts {
            let cand: String = chars[start..start + k].iter().collect();
            let cand = cand.trim_end();
            if cand.is_empty() {
                continue;
            }
            if after_get
                && let Some((f, _)) = GET_FLAGS.iter().find(|(_, n)| n.eq_ignore_ascii_case(cand))
            {
                found = Some((k, vec![0x9C, *f], true));
                break;
            }
            if !cand.contains(' ')
                && let Some(t) = lookup_name(cand)
            {
                let value = matches!(t[..], [0x22] | [0x23] | [0x97] | [0xFA, 0]);
                found = Some((k, t, value));
                break;
            }
            let (to, name) = match cand.split_once("::") {
                Some((t, n)) => (Some(t.trim()), n.trim()),
                None => (None, cand),
            };
            if let Some((to, f)) = r.field(to, name) {
                found = Some((k, field_token(context, to, f), true));
                break;
            }
        }
        // A Let variable: `0B len name 00 index`.
        // ponytail: any other single word is taken as a Let variable when
        // the calculation uses Let; the index byte (0 in every template,
        // all single-binding) is assumed to be the order of first use.
        if found.is_none() && has_let {
            let w = words.split(' ').next().unwrap_or_default();
            if !w.is_empty() {
                let idx = match let_vars.iter().position(|v| v == w) {
                    Some(x) => x,
                    None => {
                        let_vars.push(w.to_string());
                        let_vars.len() - 1
                    }
                };
                let mut t = text_token(0x0B, w);
                t.push(idx as u8);
                found = Some((w.chars().count(), t, true));
            }
        }
        let Some((k, t, value)) = found else {
            return err(start, format!("unknown name: {}", words.trim()));
        };
        after_get = t == [0x9B];
        out.extend(t);
        operand = value;
        i = start + k;
        // Give back the spaces after the name to the whitespace scanner.
        while i > start && chars[i - 1] == ' ' {
            i -= 1;
        }
    }
    Ok(out)
}

/// One token of a key-5 stream, without whitespace.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Literal, variable, field or Let name: its bytes as stored.
    Atom(Vec<u8>),
    /// Operator or punctuation byte.
    Op(u8),
    /// Function token (1 or 2 bytes).
    Func(Vec<u8>),
    /// `Get` flag (`9C xx`).
    Flag(u8),
}

/// Byte length of a text run of `units` UTF-16 units at `start`, which
/// must be followed by 00.
fn run_len(b: &[u8], start: usize, units: usize) -> Option<usize> {
    crate::calc::run(b, start, units).map(|(_, n)| n)
}

/// Splits a key-5 stream into tokens (whitespace dropped). `Err(offset)`
/// on an unknown byte.
fn scan(b: &[u8]) -> Result<Vec<Tok>, usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let t = b[i];
        let len = match t {
            0x13 => 3 + run_len(b, i + 2, *b.get(i + 1).ok_or(i)? as usize).ok_or(i)?,
            0x0C => 4 + run_len(b, i + 3, *b.get(i + 2).ok_or(i)? as usize).ok_or(i)?,
            0x0B => 4 + run_len(b, i + 2, *b.get(i + 1).ok_or(i)? as usize).ok_or(i)?,
            0x1A => {
                let units = *b.get(i + 1).ok_or(i)? as usize;
                2 + (units..=units * 3 + 3)
                    .find(|n| {
                        b.get(i + 2..i + 2 + n)
                            .is_some_and(|s| crate::text(s).encode_utf16().count() == units)
                    })
                    .ok_or(i)?
            }
            0x10 => 20,
            0x16 => 10 + *b.get(i + 7).ok_or(i)? as usize,
            0x9C | 0xF8..=0xFE => 2,
            _ => 1,
        };
        let raw = b.get(i..i + len).ok_or(i)?.to_vec();
        match t {
            0x0C => {}
            0x13 | 0x0B | 0x1A | 0x10 | 0x16 => out.push(Tok::Atom(raw)),
            0x9C => out.push(Tok::Flag(raw[1])),
            0xF8..=0xFE => out.push(Tok::Func(raw)),
            0x22 | 0x23 => out.push(Tok::Atom(raw)),
            _ if OPS.iter().any(|(o, _)| *o == t) => out.push(Tok::Op(t)),
            _ if FUNCTIONS.iter().any(|(f, _)| *f == t) => out.push(Tok::Func(raw)),
            _ => return Err(i),
        }
        i += len;
    }
    Ok(out)
}

/// Functions that take any number of arguments: their compiled token is
/// followed by a u16 argument count.
fn variadic(f: &[u8]) -> bool {
    let name = match f {
        [t] => FUNCTIONS.iter().find(|(x, _)| x == t).map(|(_, n)| *n),
        [p, x] => FUNCTIONS2
            .iter()
            .find(|(a, b, _)| a == p && b == x)
            .map(|(_, _, n)| *n),
        _ => None,
    };
    // ponytail: from the templates (Sum, List, Case…); other functions with
    // optional arguments may need a count too.
    matches!(
        name,
        Some(
            "Case"
                | "Choose"
                | "List"
                | "Sum"
                | "Count"
                | "Average"
                | "Max"
                | "Min"
                | "StDev"
                | "StDevP"
                | "Variance"
                | "VarianceP"
                | "ExecuteSQL"
                | "ExecuteSQLe"
                | "JSONSetElement"
                | "Evaluate"
        )
    )
}

/// Aggregates: field arguments are written `17` (all values).
fn aggregate(f: &[u8]) -> bool {
    let [t] = f else { return false };
    FUNCTIONS
        .iter()
        .find(|(x, _)| x == t)
        .is_some_and(|(_, n)| {
            matches!(
                *n,
                "Count"
                    | "Sum"
                    | "Average"
                    | "Max"
                    | "Min"
                    | "List"
                    | "StDev"
                    | "StDevP"
                    | "Variance"
                    | "VarianceP"
            )
        })
}

fn level(op: u8) -> Option<u8> {
    Some(match op {
        0x20 | 0x21 => 1,
        0x1F => 2,
        0x41..=0x49 => 3,
        0x50 => 4,
        0x25 | 0x26 => 5,
        0x27 | 0x28 => 6,
        0x29 => 7,
        _ => return None,
    })
}

struct Prefix {
    toks: Vec<Tok>,
    i: usize,
    /// Let variables in scope (their `0B len name 00` bytes), by binding.
    scope: Vec<Vec<u8>>,
}

impl Prefix {
    /// `Let ( name = v ; body )` or `Let ( [ a = 1 ; b = 2 ] ; body )`:
    /// `DF`, u16 binding count, the values, then the body.
    /// ponytail: binding indexes are counted from the outermost Let in
    /// scope; nested Lets are not in the templates.
    fn let_call(&mut self) -> Result<Vec<u8>, usize> {
        self.eat(0x04)?;
        let list = self.peek() == Some(&Tok::Op(0x06));
        if list {
            self.i += 1;
        }
        let mark = self.scope.len();
        let mut values = Vec::new();
        loop {
            let Some(Tok::Atom(a)) = self.peek().cloned() else {
                return Err(self.i);
            };
            if a.first() != Some(&0x0B) {
                return Err(self.i);
            }
            self.i += 1;
            self.eat(0x44)?;
            values.extend(self.expr(0)?);
            self.scope.push(a[..a.len() - 1].to_vec());
            if !list {
                break;
            }
            match self.peek() {
                Some(Tok::Op(0x03)) => self.i += 1,
                _ => break,
            }
        }
        if list {
            self.eat(0x07)?;
        }
        self.eat(0x03)?;
        let body = self.expr(0)?;
        self.eat(0x05)?;
        let mut out = vec![0xDF];
        out.extend(((self.scope.len() - mark) as u16).to_be_bytes());
        out.extend(values);
        out.extend(body);
        self.scope.truncate(mark);
        Ok(out)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn eat(&mut self, op: u8) -> Result<(), usize> {
        match self.peek() {
            Some(Tok::Op(o)) if *o == op => {
                self.i += 1;
                Ok(())
            }
            _ => Err(self.i),
        }
    }

    fn expr(&mut self, min: u8) -> Result<Vec<u8>, usize> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Op(op)) = self.peek() {
            let op = *op;
            let Some(l) = level(op).filter(|l| *l >= min) else {
                break;
            };
            self.i += 1;
            let rhs = self.expr(l + 1)?;
            let mut out = vec![op];
            out.extend(lhs);
            out.extend(rhs);
            lhs = out;
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Vec<u8>, usize> {
        if let Some(Tok::Op(op @ (0x2C | 0x1E))) = self.peek() {
            let op = *op;
            self.i += 1;
            let mut out = vec![op];
            out.extend(self.unary()?);
            return Ok(out);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Vec<u8>, usize> {
        let at = self.i;
        let t = self.peek().cloned().ok_or(at)?;
        self.i += 1;
        let mut out = match t {
            Tok::Atom(a) if a[0] == 0x10 => a[..18].to_vec(),
            Tok::Atom(a) if a[0] == 0x0B => {
                // A Let variable: `18` + u16 index of its binding.
                let name = &a[..a.len() - 1];
                let i = self
                    .scope
                    .iter()
                    .rposition(|n| n.as_slice() == name)
                    .ok_or(at)?;
                let mut out = vec![0x18];
                out.extend((i as u16).to_be_bytes());
                out
            }
            Tok::Atom(a) => a,
            Tok::Op(0x04) => {
                let e = self.expr(0)?;
                self.eat(0x05)?;
                e
            }
            Tok::Func(f) if f == [0x9B] => {
                self.eat(0x04)?;
                let Some(Tok::Flag(x)) = self.peek().cloned() else {
                    return Err(self.i);
                };
                self.i += 1;
                self.eat(0x05)?;
                vec![0x9B, x]
            }
            Tok::Func(f) if f == [0xDF] => self.let_call()?,
            Tok::Func(f) => {
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::Op(0x04)) {
                    self.i += 1;
                    if self.peek() != Some(&Tok::Op(0x05)) {
                        loop {
                            let start = self.i;
                            let mut a = self.expr(0)?;
                            // A lone field argument of an aggregate: all values.
                            if aggregate(&f) && self.i == start + 1 && a.first() == Some(&0x16) {
                                a[0] = 0x17;
                            }
                            args.push(a);
                            match self.peek() {
                                Some(Tok::Op(0x03)) => self.i += 1,
                                _ => break,
                            }
                        }
                    }
                    self.eat(0x05)?;
                }
                let mut out = f.clone();
                // Case counts its test/result pairs; a default result is
                // the pair (1, default).
                if f == [0x4C] && args.len() % 2 == 1 {
                    let one = number_token("1").unwrap_or_default();
                    args.insert(args.len() - 1, one[..18].to_vec());
                }
                if variadic(&f) {
                    let n = if f == [0x4C] {
                        args.len() / 2
                    } else {
                        args.len()
                    };
                    out.extend((n as u16).to_be_bytes());
                }
                out.extend(args.into_iter().flatten());
                out
            }
            // `[ a ; b ]` (Let, Substitute lists).
            Tok::Op(0x06) => {
                // ponytail: list encoding not seen in the templates; written
                // as its items in order.
                let mut out = Vec::new();
                loop {
                    out.extend(self.expr(0)?);
                    match self.peek() {
                        Some(Tok::Op(0x03)) => self.i += 1,
                        _ => break,
                    }
                }
                self.eat(0x07)?;
                out
            }
            _ => return Err(at),
        };
        // Repetition `x[n]`.
        if self.peek() == Some(&Tok::Op(0x06)) {
            // ponytail: compiled form of a repetition not seen; written as
            // the index after the operand.
            self.i += 1;
            out.extend(self.expr(0)?);
            self.eat(0x07)?;
        }
        Ok(out)
    }
}

/// Key 6 (prefix form) of a key-5 stream. `Err(token index)` when the
/// stream does not parse.
pub fn compiled(tokens: &[u8]) -> Result<Vec<u8>, usize> {
    let toks = scan(tokens)?;
    if toks.is_empty() {
        return Ok(Vec::new());
    }
    let mut p = Prefix {
        toks,
        i: 0,
        scope: Vec::new(),
    };
    let out = p.expr(0)?;
    if p.i != p.toks.len() {
        return Err(p.i);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    struct R;
    impl Resolve for R {
        fn field(&self, to: Option<&str>, name: &str) -> Option<(u32, u32)> {
            match (to, name) {
                (Some("Tasks"), "Status") | (None, "Status") => Some((130, 7)),
                (Some("Assignees"), "PrimaryKey") => Some((132, 11)),
                _ => None,
            }
        }
    }

    #[test]
    fn compiled_pairs_from_templates() {
        for (k5, k6) in [
            ("9b0c13017a00040c13017a009c3e0c13017a0005", "9b3e"),
            (
                "87040c13017a0016040403d0000402010b000105",
                "87000117040403d0000402010b0001",
            ),
            (
                "1a027e220c13017a00500c13017a0013012300",
                "501a027e2213012300",
            ),
            (
                "4e041a127e7e2a3b283f342e0a2833373b2823113f23050c13017a00460c13017a001002000100100000000100000000000000000020",
                "464e1a127e7e2a3b283f342e0a2833373b2823113f23100200010010000000010000000000000000",
            ),
            (
                "fa00441002000100100000000000000000000000000020",
                "44fa00100200010010000000000000000000000000",
            ),
        ] {
            assert_eq!(compiled(&hex(k5)).unwrap(), hex(k6), "{k5}");
        }
    }

    #[test]
    fn source_round_trip() {
        let src = "Get ( ScriptParameter )";
        let c = compile(src, 0, &R).unwrap();
        assert_eq!(c.tokens, hex("9b0c13017a00040c13017a009c3e0c13017a0005"));
        let src = "List ( Assignees::PrimaryKey )";
        let c = compile(src, 0, &R).unwrap();
        assert_eq!(c.compiled, hex("87000117040403d0000402010b0001"));
        let c = compile("$x & \"y\"", 0, &R).unwrap();
        assert_eq!(c.tokens, hex("1a027e220c13017a00500c13017a0013012300"));
        let c = compile("Tasks::Status = 1.5 and not IsEmpty ( $a )", 0, &R).unwrap();
        assert_eq!(c.compiled[0], 0x1F);
        assert!(compile("Nope::X", 0, &R).is_err());
        let c = compile("-2 - 1", 0, &R).unwrap();
        assert_eq!(c.compiled[..2], [0x26, 0x2C]);
    }
}
