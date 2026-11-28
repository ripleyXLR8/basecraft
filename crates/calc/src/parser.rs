//! Precedence-climbing parser producing [`Expr`].

use crate::ast::{BinOp, Expr, PREFIX_LEVEL, UnOp};
use crate::error::{ParseError, code};
use crate::lexer::{Tok, Token, lex};

/// Functions written without parentheses.
pub fn is_bare_function(name: &str) -> bool {
    ["Pi", "Random", "Self"]
        .iter()
        .any(|f| f.eq_ignore_ascii_case(name))
}

pub fn parse(src: &str) -> Result<Expr, ParseError> {
    parse_with(src, &|_| false)
}

/// [`parse`] where `custom(name)` tells the names of the file's custom
/// functions, which are called like built-in ones.
pub fn parse_with(src: &str, custom: &dyn Fn(&str) -> bool) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        i: 0,
        depth: 0,
        custom,
    };
    if p.peek() == &Tok::Eof {
        return p.fail(code::OPERAND_EXPECTED, "empty calculation");
    }
    let e = p.expr(1)?;
    match p.peek() {
        Tok::Eof => Ok(e),
        _ => p.fail(code::OPERATOR_EXPECTED, "an operator is expected here"),
    }
}

const MAX_DEPTH: usize = 200;

struct Parser<'a> {
    toks: Vec<Token>,
    i: usize,
    depth: usize,
    custom: &'a dyn Fn(&str) -> bool,
}

fn keyword(t: &Tok) -> Option<&'static str> {
    if let Tok::Ident {
        name,
        quoted: false,
    } = t
    {
        ["and", "or", "xor", "not"]
            .into_iter()
            .find(|k| k.eq_ignore_ascii_case(name))
    } else {
        None
    }
}

fn binop(t: &Tok) -> Option<BinOp> {
    use BinOp::*;
    Some(match t {
        Tok::Sym(s) => match *s {
            "+" => Add,
            "-" => Sub,
            "*" => Mul,
            "/" => Div,
            "^" => Pow,
            "&" => Concat,
            "=" => Eq,
            "≠" => Ne,
            "<" => Lt,
            "≤" => Le,
            ">" => Gt,
            "≥" => Ge,
            _ => return None,
        },
        _ => match keyword(t)? {
            "and" => And,
            "or" => Or,
            "xor" => Xor,
            _ => return None,
        },
    })
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn next(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn fail<T>(&self, code: u32, message: &str) -> Result<T, ParseError> {
        Err(ParseError {
            pos: self.toks[self.i].pos,
            code,
            message: message.into(),
        })
    }

    fn expect(&mut self, sym: &'static str) -> Result<(), ParseError> {
        if self.peek() == &Tok::Sym(sym) {
            self.next();
            return Ok(());
        }
        match (sym, self.peek()) {
            (")", Tok::Eof) => self.fail(code::UNBALANCED_PAREN, "unbalanced parenthesis"),
            (")", _) => self.fail(code::OPERATOR_MISSING, "operator or \";\" expected"),
            _ => self.fail(code::OPERATOR_MISSING, &format!("\"{sym}\" expected")),
        }
    }

    fn expr(&mut self, min: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        while let Some(op) = binop(self.peek()) {
            if op.level() < min {
                break;
            }
            self.next();
            let rhs = self.expr(op.level() + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Every level of nesting passes through here, so this is where the
    /// depth limit lives (keeps the recursive parser and evaluator off the
    /// end of the stack).
    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.depth >= MAX_DEPTH {
            return self.fail(code::GENERIC, "formula is nested too deeply");
        }
        self.depth += 1;
        let r = self.unary_inner();
        self.depth -= 1;
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, ParseError> {
        let op = match self.peek() {
            Tok::Sym("-") => Some(UnOp::Neg),
            Tok::Sym("+") => Some(UnOp::Plus),
            t if keyword(t) == Some("not") => Some(UnOp::Not),
            _ => None,
        };
        match op {
            Some(op) => {
                self.next();
                let e = self.unary()?;
                debug_assert!(PREFIX_LEVEL > BinOp::Pow.level());
                Ok(Expr::Unary(op, Box::new(e)))
            }
            None => self.primary(),
        }
    }

    /// `[expr]` right after a field or variable name.
    fn repetition(&mut self) -> Result<Option<Box<Expr>>, ParseError> {
        if self.peek() != &Tok::Sym("[") {
            return Ok(None);
        }
        self.next();
        let e = self.expr(1)?;
        self.expect("]")?;
        Ok(Some(Box::new(e)))
    }

    fn items(&mut self, close: &'static str) -> Result<Vec<Expr>, ParseError> {
        let mut items = Vec::new();
        if self.peek() == &Tok::Sym(close) {
            self.next();
            return Ok(items);
        }
        loop {
            items.push(self.expr(1)?);
            if self.peek() == &Tok::Sym(";") {
                self.next();
                continue;
            }
            self.expect(close)?;
            return Ok(items);
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.next() {
            Tok::Num(n) => Ok(Expr::Number(n)),
            Tok::Str(s) => Ok(Expr::Text(s)),
            Tok::Sym("(") => {
                let e = self.expr(1)?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Sym("[") => Ok(Expr::List(self.items("]")?)),
            Tok::Var(name) => Ok(Expr::Var {
                name,
                rep: self.repetition()?,
            }),
            Tok::Ident { name, quoted } => {
                if !quoted
                    && keyword(&Tok::Ident {
                        name: name.clone(),
                        quoted,
                    })
                    .is_some()
                {
                    self.i -= 1;
                    return self.fail(code::OPERAND_EXPECTED, "operand expected");
                }
                // Pi, Random and Self take no parentheses: Pi ( ) is 1212.
                if !quoted && is_bare_function(&name) {
                    return Ok(Expr::Call { name, args: vec![] });
                }
                if !quoted && self.peek() == &Tok::Sym("(") {
                    if !crate::functions::exists(&name) && !(self.custom)(&name) {
                        self.i -= 1;
                        return self.fail(code::OPERATOR_MISSING, "function not found");
                    }
                    self.next();
                    let args = self.items(")")?;
                    return Ok(Expr::Call { name, args });
                }
                if self.peek() == &Tok::Sym("::") {
                    self.next();
                    let Tok::Ident { name: field, .. } = self.peek().clone() else {
                        return self.fail(code::NAME_MISSING, "field name is missing");
                    };
                    self.next();
                    return Ok(Expr::Field {
                        table: Some(name),
                        name: field,
                        rep: self.repetition()?,
                    });
                }
                Ok(Expr::Field {
                    table: None,
                    name,
                    rep: self.repetition()?,
                })
            }
            Tok::Eof => {
                self.i = self.toks.len() - 1;
                self.fail(
                    code::OPERAND_EXPECTED,
                    "number, text constant, field name or \"(\" expected",
                )
            }
            _ => {
                self.i -= 1;
                self.fail(
                    code::OPERAND_EXPECTED,
                    "number, text constant, field name or \"(\" expected",
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(src: &str) {
        let a = parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        let printed = a.to_string();
        let b = parse(&printed).unwrap_or_else(|e| panic!("{printed}: {e}"));
        assert_eq!(a, b, "{src} -> {printed}");
    }

    #[test]
    fn round_trips() {
        for src in [
            "1 + 2 * 3",
            "(1 + 2) * 3",
            "1 - (2 - 3)",
            "1 - 2 - 3",
            "2 ^ 3 ^ 2",
            "2 ^ (3 ^ 2)",
            "-2 ^ 2",
            "-(2 ^ 2)",
            "not a = b",
            "not (a = b)",
            "a and b or c xor d",
            "a and (b or c)",
            "\"a\" & ¶ & \"b\\\"c\\\\d\\¶\"",
            "Let ( [ x = 1 ; y = x + 1 ] ; x & y )",
            "Substitute ( t ; [ \"a\" ; \"b\" ] ; [ \"c\" ; \"d\" ] )",
            "Contacts::First[2] + $x[3] + $$Y",
            "${weird name} & T::${odd-field}",
            "Pi * 2 + Random",
            "Get ( CurrentDate ) - 1",
            "If ( x ≥ 1 ; \"y\" ; \"n\" ) /* comment */ // line",
            ".5 + 0.25",
            "a <> b and c <= d and e >= f",
            "- -1",
        ] {
            round_trip(src);
        }
    }

    #[test]
    fn precedence() {
        assert_eq!(parse("1+2*3").unwrap().to_string(), "1 + 2 * 3");
        assert_eq!(parse("(1+2)*3").unwrap().to_string(), "(1 + 2) * 3");
        assert_eq!(parse("-2^2").unwrap().to_string(), "-2 ^ 2");
        assert_eq!(parse("a & b = c").unwrap().to_string(), "a & b = c");
        assert!(parse("1,2").is_err());
        assert_eq!(parse("left(1,2)").unwrap().to_string(), "left ( 1 ; 2 )");
    }

    #[test]
    fn errors() {
        let e = |s: &str| parse(s).unwrap_err();
        assert_eq!(e("1 +").code, code::OPERAND_EXPECTED);
        assert_eq!(e("1 +").pos, 3);
        assert_eq!(e("(1 + 2").code, code::UNBALANCED_PAREN);
        assert_eq!(e("1 + 2)").code, code::OPERATOR_EXPECTED);
        assert_eq!(e("NoSuchFunction ( 1 )").code, code::OPERATOR_MISSING);
        assert_eq!(e("\"abc").code, code::UNTERMINATED_TEXT);
        assert_eq!(e("1 /* x").code, code::UNTERMINATED_COMMENT);
        assert_eq!(e("1 2").code, code::OPERATOR_EXPECTED);
        assert_eq!(e("1 2").pos, 2);
        assert_eq!(e("").code, code::OPERAND_EXPECTED);
        assert_eq!(e("If(1;)").pos, 5);
    }
}
