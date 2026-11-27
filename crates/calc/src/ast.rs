//! Syntax tree and its pretty-printer. Printing then re-parsing yields the
//! same tree (parentheses are emitted only where precedence needs them).

use crate::num::{self, Num};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(Num),
    Text(String),
    /// `Table::Field`, `Field`, `Field[2]`. A bare name (no table, no
    /// repetition) may also be a Let variable or a constant like JSONString;
    /// the evaluator decides.
    Field {
        table: Option<String>,
        name: String,
        rep: Option<Box<Expr>>,
    },
    /// `$local` or `$$global`; `name` keeps the dollar signs.
    Var {
        name: String,
        rep: Option<Box<Expr>>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// `[a ; b ; c]` as used by Let, While and Substitute.
    List(Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
}

/// Binding strength, higher binds tighter. Prefix operators sit above `^`
/// (FileMaker lists `not` before `^`), so `-2^2` is `(-2)^2`.
pub const PREFIX_LEVEL: u8 = 8;

impl BinOp {
    pub fn level(self) -> u8 {
        use BinOp::*;
        match self {
            Or | Xor => 1,
            And => 2,
            Eq | Ne | Lt | Le | Gt | Ge => 3,
            Concat => 4,
            Add | Sub => 5,
            Mul | Div => 6,
            Pow => 7,
        }
    }

    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Pow => "^",
            Concat => "&",
            Eq => "=",
            Ne => "≠",
            Lt => "<",
            Le => "≤",
            Gt => ">",
            Ge => "≥",
            And => "and",
            Or => "or",
            Xor => "xor",
        }
    }
}

impl Expr {
    fn level(&self) -> u8 {
        match self {
            Expr::Binary(op, ..) => op.level(),
            Expr::Unary(..) => PREFIX_LEVEL,
            _ => u8::MAX,
        }
    }
}

const KEYWORDS: [&str; 4] = ["and", "or", "xor", "not"];

pub fn is_plain_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_alphabetic() || f == '_')
        && c.all(|ch| ch.is_alphanumeric() || ch == '_')
        && !KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(s))
}

fn write_name(f: &mut fmt::Formatter, s: &str) -> fmt::Result {
    if is_plain_name(s) {
        f.write_str(s)
    } else {
        write!(f, "${{{s}}}")
    }
}

fn write_rep(f: &mut fmt::Formatter, rep: &Option<Box<Expr>>) -> fmt::Result {
    match rep {
        Some(r) => write!(f, "[{r}]"),
        None => Ok(()),
    }
}

fn write_child(f: &mut fmt::Formatter, e: &Expr, paren: bool) -> fmt::Result {
    if paren {
        write!(f, "({e})")
    } else {
        write!(f, "{e}")
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Expr::Number(n) => f.write_str(&num::format(n)),
            Expr::Text(s) => {
                f.write_str("\"")?;
                for c in s.chars() {
                    match c {
                        '"' => f.write_str("\\\"")?,
                        '\\' => f.write_str("\\\\")?,
                        '¶' => f.write_str("\\¶")?,
                        '\r' => f.write_str("¶")?,
                        c => write!(f, "{c}")?,
                    }
                }
                f.write_str("\"")
            }
            Expr::Field { table, name, rep } => {
                if let Some(t) = table {
                    write_name(f, t)?;
                    f.write_str("::")?;
                }
                write_name(f, name)?;
                write_rep(f, rep)
            }
            Expr::Var { name, rep } => {
                f.write_str(name)?;
                write_rep(f, rep)
            }
            Expr::Call { name, args } => {
                if args.is_empty() && crate::parser::is_bare_function(name) {
                    return f.write_str(name);
                }
                write!(f, "{name} ( ")?;
                write_list(f, args)?;
                f.write_str(" )")
            }
            Expr::List(items) => {
                f.write_str("[ ")?;
                write_list(f, items)?;
                f.write_str(" ]")
            }
            Expr::Unary(op, e) => {
                f.write_str(match op {
                    UnOp::Neg => "-",
                    UnOp::Plus => "+",
                    UnOp::Not => "not ",
                })?;
                write_child(f, e, e.level() <= PREFIX_LEVEL)
            }
            Expr::Binary(op, l, r) => {
                write_child(f, l, l.level() < op.level())?;
                write!(f, " {} ", op.symbol())?;
                write_child(f, r, r.level() <= op.level())
            }
        }
    }
}

fn write_list(f: &mut fmt::Formatter, items: &[Expr]) -> fmt::Result {
    for (i, a) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(" ; ")?;
        }
        write!(f, "{a}")?;
    }
    Ok(())
}
