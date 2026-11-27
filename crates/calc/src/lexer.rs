//! Tokenizer. Positions are character offsets into the source.

use crate::error::{ParseError, code};
use crate::num::{self, Num};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(Num),
    Str(String),
    /// Identifier; `quoted` for the `${...}` form, which is never a keyword.
    Ident {
        name: String,
        quoted: bool,
    },
    /// `$name` / `$$name`, dollars included.
    Var(String),
    /// Operator or punctuation: + - * / ^ & = ≠ < ≤ > ≥ ( ) [ ] ; ::
    Sym(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub pos: usize,
}

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let s: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let err = |pos, code, msg: &str| {
        Err(ParseError {
            pos,
            code,
            message: msg.into(),
        })
    };
    while i < s.len() {
        let c = s[i];
        let start = i;
        let next = s.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && next == Some('/') {
            while i < s.len() && s[i] != '\n' && s[i] != '\r' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            i += 2;
            loop {
                if i + 1 >= s.len() {
                    return err(
                        start,
                        code::UNTERMINATED_COMMENT,
                        "comment is not terminated with */",
                    );
                }
                if s[i] == '*' && s[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let tok = if c == '"' {
            i += 1;
            let mut text = String::new();
            loop {
                match s.get(i) {
                    None => {
                        return err(
                            start,
                            code::UNTERMINATED_TEXT,
                            "text constant must end with a quotation mark",
                        );
                    }
                    Some('"') => break,
                    Some('¶') => text.push('\r'),
                    Some('\\') => match s.get(i + 1) {
                        Some(&e @ ('"' | '\\' | '¶')) => {
                            text.push(e);
                            i += 1;
                        }
                        _ => text.push('\\'),
                    },
                    Some(&ch) => text.push(ch),
                }
                i += 1;
            }
            i += 1;
            Tok::Str(text)
        } else if c == '¶' {
            i += 1;
            Tok::Str("\r".into())
        } else if c.is_ascii_digit() || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            while i < s.len() && s[i].is_ascii_digit() {
                i += 1;
            }
            if s.get(i) == Some(&'.') {
                i += 1;
                while i < s.len() && s[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // Exponent: 1e3, 1.5E-2, 1.0E+20.
            if matches!(s.get(i), Some('e' | 'E')) {
                let d = if matches!(s.get(i + 1), Some('+' | '-')) {
                    2
                } else {
                    1
                };
                if s.get(i + d).is_some_and(|c| c.is_ascii_digit()) {
                    i += d;
                    while i < s.len() && s[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = s[start..i].iter().collect();
            Tok::Num(num::parse(&format!("0{text}")).unwrap())
        } else if c == '$' && next == Some('{') {
            let close = s[i..].iter().position(|&ch| ch == '}');
            let Some(close) = close else {
                return err(start, code::NAME_MISSING, "unterminated ${ name");
            };
            let name: String = s[i + 2..i + close].iter().collect();
            i += close + 1;
            Tok::Ident { name, quoted: true }
        } else if c == '$' {
            i += 1;
            if s.get(i) == Some(&'$') {
                i += 1;
            }
            while i < s.len() && ident_char(s[i]) {
                i += 1;
            }
            // A bare "$" is a variable with no name: always empty.
            Tok::Var(s[start..i].iter().collect())
        } else if ident_start(c) {
            while i < s.len() && ident_char(s[i]) {
                i += 1;
            }
            Tok::Ident {
                name: s[start..i].iter().collect(),
                quoted: false,
            }
        } else {
            let two: String = s[i..(i + 2).min(s.len())].iter().collect();
            let (sym, len) = match (two.as_str(), c) {
                ("<>", _) => ("≠", 2),
                ("<=", _) => ("≤", 2),
                (">=", _) => ("≥", 2),
                ("::", _) => ("::", 2),
                (_, '+') => ("+", 1),
                (_, '-') => ("-", 1),
                (_, '*') => ("*", 1),
                (_, '/') => ("/", 1),
                (_, '^') => ("^", 1),
                (_, '&') => ("&", 1),
                (_, '=') => ("=", 1),
                (_, '≠') => ("≠", 1),
                (_, '<') => ("<", 1),
                (_, '>') => (">", 1),
                (_, '≤') => ("≤", 1),
                (_, '≥') => ("≥", 1),
                (_, '(') => ("(", 1),
                (_, ')') => (")", 1),
                (_, '[') => ("[", 1),
                (_, ']') => ("]", 1),
                (_, ';' | ',') => (";", 1),
                _ => {
                    return err(
                        start,
                        code::OPERAND_EXPECTED,
                        &format!("unexpected character '{c}'"),
                    );
                }
            };
            i += len;
            Tok::Sym(sym)
        };
        out.push(Token { tok, pos: start });
    }
    out.push(Token {
        tok: Tok::Eof,
        pos: s.len(),
    });
    Ok(out)
}
