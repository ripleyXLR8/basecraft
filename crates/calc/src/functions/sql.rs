//! ExecuteSQL: a minimal SELECT over the rows a host returns from
//! [`Context::rows`](crate::Context::rows).
//!
//! Supported: `SELECT [DISTINCT] * | t.* | expr [AS name], ... FROM t [alias]
//! {, t | [INNER | LEFT [OUTER]] JOIN t [alias] ON cond} [WHERE ...]
//! [GROUP BY col, ...] [HAVING ...] [ORDER BY expr|n|name [ASC|DESC], ...]
//! [OFFSET n ROWS] [FETCH FIRST n ROWS ONLY]`, aggregates `COUNT(*)`,
//! `COUNT([DISTINCT] e)`, `SUM`, `AVG`, `MIN`, `MAX`, with `= <> != < <= > >=`, `LIKE`, `IN`,
//! `BETWEEN`, `IS [NOT] NULL`, `AND`, `OR`, `NOT`, `?` parameters and
//! `DATE 'yyyy-mm-dd'` / `TIME` / `TIMESTAMP` literals. Text comparisons are
//! case-sensitive and an empty value is NULL, as in FileMaker's SQL.
//! Expressions: `+ - * /` (`+` and `||` join text; `-` joins text moving
//! the left side's trailing blanks to the end; date ± days), `CASE`,
//! `CAST ( e AS type )`, the scalar functions in [`FUNCS`] and [`NOW`]
//! (Claris SQL Reference), `( SELECT … )` and `IN ( SELECT … )`
//! subqueries, and the system tables FileMaker_Tables, FileMaker_Fields,
//! FileMaker_BaseTables and FileMaker_BaseTableFields.
// ponytail: subqueries run once (no correlated subqueries, no EXISTS);
// ModCount is NULL (BaseCraft does not count design changes); no
// FileMaker_ValueLists. Add when files use them.

use super::txt;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num;
use crate::schema::Rows;
use crate::value::{self, Value, civil};
use bigdecimal::ToPrimitive;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Str(String),
    Num(String),
    Sym(&'static str),
    Param,
}

fn err(msg: impl Into<String>) -> EvalError {
    EvalError::new(code::SQL_ERROR, msg)
}

fn lex(s: &str) -> Result<Vec<Tok>, EvalError> {
    let c: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '\'' || ch == '"' {
            let mut v = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None => return Err(err("unterminated quote")),
                    Some(&q) if q == ch && c.get(i + 1) == Some(&ch) => {
                        v.push(ch);
                        i += 2;
                    }
                    Some(&q) if q == ch => {
                        i += 1;
                        break;
                    }
                    Some(&x) => {
                        v.push(x);
                        i += 1;
                    }
                }
            }
            out.push(if ch == '\'' {
                Tok::Str(v)
            } else {
                Tok::Quoted(v)
            });
        } else if ch.is_ascii_digit()
            || (ch == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(c[start..i].iter().collect()));
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Word(c[start..i].iter().collect()));
        } else if ch == '?' {
            out.push(Tok::Param);
            i += 1;
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            let sym = ["<>", "!=", "<=", ">=", "||"]
                .into_iter()
                .find(|s| *s == two)
                .or_else(|| {
                    ["=", "<", ">", "(", ")", ",", "*", ".", "+", "-", "/"]
                        .into_iter()
                        .find(|s| s.starts_with(ch))
                })
                .ok_or_else(|| err(format!("unexpected character {ch}")))?;
            i += sym.len();
            out.push(Tok::Sym(sym));
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Agg {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone)]
enum Ex {
    /// `[qualifier.]column`, bound to [`Ex::Idx`] before evaluation.
    Col(Option<String>, String),
    /// Column of the joined row.
    Idx(usize),
    Lit(Value),
    /// Aggregate over a group: `COUNT(*)` has no argument; DISTINCT.
    Agg(Agg, Option<Box<Ex>>, bool),
    Cmp(&'static str, Box<Ex>, Box<Ex>),
    Like(Box<Ex>, Box<Ex>, bool),
    In(Box<Ex>, Vec<Ex>, bool),
    Between(Box<Ex>, Box<Ex>, Box<Ex>, bool),
    Null(Box<Ex>, bool),
    And(Box<Ex>, Box<Ex>),
    Or(Box<Ex>, Box<Ex>),
    Not(Box<Ex>),
    /// A scalar function or operator (upper-case name; `CAST` has the
    /// type as a second, literal argument).
    Func(String, Vec<Ex>),
    /// `CASE [input] WHEN … THEN … [ELSE …] END`.
    Case(Option<Box<Ex>>, Vec<(Ex, Ex)>, Option<Box<Ex>>),
    /// A subquery, run before the query: its first value, or every value
    /// of its first column in an IN list.
    Select(Box<Query>),
}

#[derive(Debug, Clone)]
enum Item {
    /// `*` or `t.*`.
    All(Option<String>),
    /// An expression and its `AS` name.
    Ex(Ex, Option<String>),
}

/// A table of the FROM clause: its name, the name it goes by, and how it
/// joins the tables before it (LEFT, ON condition; none: a cross join).
#[derive(Debug, Clone)]
struct From {
    name: String,
    alias: String,
    left: bool,
    on: Option<Ex>,
}

#[derive(Debug, Clone)]
struct Query {
    distinct: bool,
    items: Vec<Item>,
    from: Vec<From>,
    filter: Option<Ex>,
    group: Vec<Ex>,
    having: Option<Ex>,
    order: Vec<(Ex, bool)>,
    offset: usize,
    limit: Option<usize>,
}

/// Words that end a table reference (no alias).
const CLAUSES: [&str; 11] = [
    "WHERE", "ORDER", "OFFSET", "FETCH", "GROUP", "HAVING", "LEFT", "INNER", "JOIN", "ON", "OUTER",
];

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    params: &'a [Value],
    next_param: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    /// The next word is `k` (not taken).
    fn at(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(k))
    }

    fn kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(k)) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, k: &str) -> Result<(), EvalError> {
        if self.kw(k) {
            Ok(())
        } else {
            Err(err(format!("{k} expected")))
        }
    }

    fn expect(&mut self, s: &str) -> Result<(), EvalError> {
        if self.sym(s) {
            Ok(())
        } else {
            Err(err(format!("{s} expected")))
        }
    }

    fn name(&mut self) -> Result<String, EvalError> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Word(w) | Tok::Quoted(w)) => {
                self.i += 1;
                Ok(w)
            }
            _ => Err(err("name expected")),
        }
    }

    /// `col` or `table.col`.
    fn column(&mut self) -> Result<Ex, EvalError> {
        let n = self.name()?;
        if self.sym(".") {
            return Ok(Ex::Col(Some(n), self.name()?));
        }
        Ok(Ex::Col(None, n))
    }

    fn number(&mut self) -> Result<usize, EvalError> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                n.parse().map_err(|_| err("integer expected"))
            }
            Some(Tok::Param) => {
                self.i += 1;
                let v = self.param()?;
                usize::try_from(crate::eval::to_index(&v)?).map_err(|_| err("bad count"))
            }
            _ => Err(err("number expected")),
        }
    }

    fn param(&mut self) -> Result<Value, EvalError> {
        let v = self
            .params
            .get(self.next_param)
            .cloned()
            .ok_or_else(|| err("missing ? argument"))?;
        self.next_param += 1;
        Ok(v)
    }

    /// `[AS] name`, unless the next word is one of `stop`.
    fn alias(&mut self, stop: &[&str]) -> Result<Option<String>, EvalError> {
        if !self.kw("AS") {
            match self.peek() {
                Some(Tok::Word(w)) if !stop.iter().any(|k| w.eq_ignore_ascii_case(k)) => {}
                Some(Tok::Quoted(_)) => {}
                _ => return Ok(None),
            }
        }
        self.name().map(Some)
    }

    /// `name [[AS] alias]`.
    fn table(&mut self, left: bool, on: Option<Ex>) -> Result<From, EvalError> {
        let name = self.name()?;
        let alias = self.alias(&CLAUSES)?.unwrap_or_else(|| name.clone());
        Ok(From {
            name,
            alias,
            left,
            on,
        })
    }

    fn query(&mut self) -> Result<Query, EvalError> {
        self.expect_kw("SELECT")?;
        let distinct = self.kw("DISTINCT");
        let mut items = vec![];
        loop {
            let star = matches!(
                (self.peek(), self.t.get(self.i + 1), self.t.get(self.i + 2)),
                (
                    Some(Tok::Word(_) | Tok::Quoted(_)),
                    Some(Tok::Sym(".")),
                    Some(Tok::Sym("*"))
                )
            );
            if self.sym("*") {
                items.push(Item::All(None));
            } else if star {
                let t = self.name()?;
                self.i += 2;
                items.push(Item::All(Some(t)));
            } else {
                let e = self.or()?;
                let alias = self.alias(&["FROM"])?;
                items.push(Item::Ex(e, alias));
            }
            if !self.sym(",") {
                break;
            }
        }
        self.expect_kw("FROM")?;
        let mut from = vec![self.table(false, None)?];
        loop {
            if self.sym(",") {
                from.push(self.table(false, None)?);
                continue;
            }
            let left = self.kw("LEFT");
            if left {
                self.kw("OUTER");
                self.expect_kw("JOIN")?;
            } else if self.kw("INNER") {
                self.expect_kw("JOIN")?;
            } else if !self.kw("JOIN") {
                break;
            }
            let mut t = self.table(left, None)?;
            self.expect_kw("ON")?;
            t.on = Some(self.or()?);
            from.push(t);
        }
        let filter = if self.kw("WHERE") {
            Some(self.or()?)
        } else {
            None
        };
        let mut group = vec![];
        if self.kw("GROUP") {
            self.expect_kw("BY")?;
            loop {
                group.push(self.column()?);
                if !self.sym(",") {
                    break;
                }
            }
        }
        let having = if self.kw("HAVING") {
            Some(self.or()?)
        } else {
            None
        };
        let mut order = vec![];
        if self.kw("ORDER") {
            self.expect_kw("BY")?;
            loop {
                let key = match self.peek() {
                    Some(Tok::Num(_)) => Ex::Lit(Value::int(self.number()? as i64)),
                    _ => self.sum()?,
                };
                let desc = self.kw("DESC");
                if !desc {
                    self.kw("ASC");
                }
                order.push((key, desc));
                if !self.sym(",") {
                    break;
                }
            }
        }
        let mut offset = 0;
        if self.kw("OFFSET") {
            offset = self.number()?;
            let _ = self.kw("ROWS") || self.kw("ROW");
        }
        let mut limit = None;
        if self.kw("FETCH") {
            let _ = self.kw("FIRST") || self.kw("NEXT");
            limit = Some(if matches!(self.peek(), Some(Tok::Num(_) | Tok::Param)) {
                self.number()?
            } else {
                1
            });
            let _ = self.kw("ROWS") || self.kw("ROW");
            self.expect_kw("ONLY")?;
        }
        Ok(Query {
            distinct,
            items,
            from,
            filter,
            group,
            having,
            order,
            offset,
            limit,
        })
    }

    fn or(&mut self) -> Result<Ex, EvalError> {
        let mut l = self.and()?;
        while self.kw("OR") {
            l = Ex::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Ex, EvalError> {
        let mut l = self.not()?;
        while self.kw("AND") {
            l = Ex::And(Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Ex, EvalError> {
        if self.kw("NOT") {
            return Ok(Ex::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Ex, EvalError> {
        let l = self.sum()?;
        if let Some(Tok::Sym(s)) = self.peek().cloned()
            && ["=", "<>", "!=", "<", "<=", ">", ">="].contains(&s)
        {
            self.i += 1;
            let r = self.sum()?;
            return Ok(Ex::Cmp(s, Box::new(l), Box::new(r)));
        }
        if self.kw("IS") {
            let neg = self.kw("NOT");
            self.expect_kw("NULL")?;
            return Ok(Ex::Null(Box::new(l), neg));
        }
        let neg = self.kw("NOT");
        if self.kw("LIKE") {
            return Ok(Ex::Like(Box::new(l), Box::new(self.sum()?), neg));
        }
        if self.kw("IN") {
            self.expect("(")?;
            let mut items = vec![];
            if self.at("SELECT") {
                items.push(Ex::Select(Box::new(self.query()?)));
            } else {
                items.push(self.sum()?);
                while self.sym(",") {
                    items.push(self.sum()?);
                }
            }
            self.expect(")")?;
            return Ok(Ex::In(Box::new(l), items, neg));
        }
        if self.kw("BETWEEN") {
            let lo = self.sum()?;
            self.expect_kw("AND")?;
            let hi = self.sum()?;
            return Ok(Ex::Between(Box::new(l), Box::new(lo), Box::new(hi), neg));
        }
        if neg {
            return Err(err("LIKE, IN or BETWEEN expected after NOT"));
        }
        Ok(l)
    }

    /// `COUNT(*)`, `COUNT([DISTINCT] e)`, `SUM`, `AVG`, `MIN`, `MAX`.
    fn aggregate(&mut self) -> Result<Option<Ex>, EvalError> {
        let f = match (self.peek(), self.t.get(self.i + 1)) {
            (Some(Tok::Word(w)), Some(Tok::Sym("("))) => match w.to_ascii_uppercase().as_str() {
                "COUNT" => Agg::Count,
                "SUM" => Agg::Sum,
                "AVG" => Agg::Avg,
                "MIN" => Agg::Min,
                "MAX" => Agg::Max,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        self.i += 2;
        if f == Agg::Count && self.sym("*") {
            self.expect(")")?;
            return Ok(Some(Ex::Agg(f, None, false)));
        }
        let distinct = self.kw("DISTINCT");
        let e = self.or()?;
        self.expect(")")?;
        Ok(Some(Ex::Agg(f, Some(Box::new(e)), distinct)))
    }

    /// `+`, `-` and `||`, left to right.
    fn sum(&mut self) -> Result<Ex, EvalError> {
        let mut l = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(s @ ("+" | "-" | "||"))) => *s,
                _ => return Ok(l),
            };
            self.i += 1;
            l = Ex::Func(op.into(), vec![l, self.term()?]);
        }
    }

    fn term(&mut self) -> Result<Ex, EvalError> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(s @ ("*" | "/"))) => *s,
                _ => return Ok(l),
            };
            self.i += 1;
            l = Ex::Func(op.into(), vec![l, self.unary()?]);
        }
    }

    fn unary(&mut self) -> Result<Ex, EvalError> {
        if self.sym("-") {
            return Ok(Ex::Func("NEG".into(), vec![self.unary()?]));
        }
        self.sym("+");
        self.operand()
    }

    /// `CASE [input] WHEN … THEN … {WHEN … THEN …} [ELSE …] END`.
    fn case(&mut self) -> Result<Ex, EvalError> {
        let input = if self.at("WHEN") {
            None
        } else {
            Some(Box::new(self.sum()?))
        };
        let mut arms = vec![];
        while self.kw("WHEN") {
            let w = self.or()?;
            self.expect_kw("THEN")?;
            arms.push((w, self.or()?));
        }
        if arms.is_empty() {
            return Err(err("WHEN expected"));
        }
        let other = if self.kw("ELSE") {
            Some(Box::new(self.or()?))
        } else {
            None
        };
        self.expect_kw("END")?;
        Ok(Ex::Case(input, arms, other))
    }

    /// A scalar function after its name and `(`.
    fn call(&mut self, name: String) -> Result<Ex, EvalError> {
        if !FUNCS.contains(&name.as_str()) && !NOW.contains(&name.as_str()) {
            return Err(err(format!("unknown function {name}")));
        }
        let mut args = vec![];
        match name.as_str() {
            "CAST" => {
                args.push(self.or()?);
                self.expect_kw("AS")?;
                let ty = self.name()?.to_ascii_uppercase();
                // VARCHAR(20), DECIMAL(10,2): the size changes nothing here.
                if self.sym("(") {
                    while !self.sym(")") {
                        if self.peek().is_none() {
                            return Err(err(") expected"));
                        }
                        self.i += 1;
                    }
                }
                args.push(Ex::Lit(Value::Text(ty)));
            }
            // POSITION ( needle IN text ), or with a comma.
            "POSITION" => {
                args.push(self.sum()?);
                if !self.kw("IN") {
                    self.expect(",")?;
                }
                args.push(self.sum()?);
            }
            _ if self.peek() == Some(&Tok::Sym(")")) => {}
            _ => loop {
                args.push(self.or()?);
                if !self.sym(",") {
                    break;
                }
            },
        }
        self.expect(")")?;
        Ok(Ex::Func(name, args))
    }

    fn operand(&mut self) -> Result<Ex, EvalError> {
        if self.sym("(") {
            if self.at("SELECT") {
                let q = self.query()?;
                self.expect(")")?;
                return Ok(Ex::Select(Box::new(q)));
            }
            let e = self.or()?;
            self.expect(")")?;
            return Ok(e);
        }
        if let Some(a) = self.aggregate()? {
            return Ok(a);
        }
        if let Some(Tok::Word(w)) = self.peek().cloned() {
            let up = w.to_ascii_uppercase();
            if up == "CASE" {
                self.i += 1;
                return self.case();
            }
            if self.t.get(self.i + 1) == Some(&Tok::Sym("(")) {
                self.i += 2;
                return self.call(up);
            }
            if NOW.contains(&up.as_str()) {
                self.i += 1;
                return Ok(Ex::Func(up, vec![]));
            }
        }
        let t = self
            .t
            .get(self.i)
            .cloned()
            .ok_or_else(|| err("operand expected"))?;
        if let Tok::Word(w) = &t {
            let parse: Option<fn(&str) -> Option<Value>> = match w.to_ascii_uppercase().as_str() {
                "DATE" => Some(|s| sql_date(s).map(Value::Date)),
                "TIME" => Some(|s| value::parse_time(s).map(Value::Time)),
                "TIMESTAMP" => Some(|s| sql_timestamp(s).map(Value::Timestamp)),
                "NULL" => {
                    self.i += 1;
                    return Ok(Ex::Lit(Value::empty()));
                }
                _ => None,
            };
            if let (Some(f), Some(Tok::Str(s))) = (parse, self.t.get(self.i + 1).cloned()) {
                self.i += 2;
                return f(&s)
                    .map(Ex::Lit)
                    .ok_or_else(|| err(format!("invalid {w} literal")));
            }
        }
        match t {
            Tok::Str(s) => {
                self.i += 1;
                Ok(Ex::Lit(Value::Text(s)))
            }
            Tok::Num(n) => {
                self.i += 1;
                num::parse(&n)
                    .map(|n| Ex::Lit(Value::Number(n)))
                    .ok_or_else(|| err("invalid number"))
            }
            Tok::Param => {
                self.i += 1;
                Ok(Ex::Lit(self.param()?))
            }
            Tok::Word(_) | Tok::Quoted(_) => self.column(),
            Tok::Sym(s) => Err(err(format!("unexpected {s}"))),
        }
    }
}

/// Scalar functions (Claris SQL Reference: functions that return character
/// strings, numbers and dates, conditional functions), evaluated by [`func`].
const FUNCS: [&str; 40] = [
    "CAST",
    "POSITION",
    "UPPER",
    "LOWER",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "LEFT",
    "RIGHT",
    "SUBSTR",
    "SUBSTRING",
    "LENGTH",
    "CHAR_LENGTH",
    "CHARACTER_LENGTH",
    "COALESCE",
    "NULLIF",
    "ROUND",
    "ABS",
    "SIGN",
    "INT",
    "FLOOR",
    "CEIL",
    "CEILING",
    "MOD",
    "SQRT",
    "CHR",
    "SPACE",
    "STRVAL",
    "NUMVAL",
    "DATEVAL",
    "TIMESTAMPVAL",
    "YEAR",
    "MONTH",
    "DAY",
    "HOUR",
    "MINUTE",
    "SECOND",
    "DAYOFWEEK",
    "DAYNAME",
    "MONTHNAME",
];

/// Functions of the clock and the session, with or without `()`; read
/// once per query ([`resolve`]).
const NOW: [&str; 11] = [
    "CURRENT_DATE",
    "CURDATE",
    "TODAY",
    "CURRENT_TIME",
    "CURTIME",
    "CURRENT_TIMESTAMP",
    "CURTIMESTAMP",
    "CURRENT_USER",
    "USER",
    "USERNAME",
    "PI",
];

/// SQL date literal: `YYYY-MM-DD`, or the file's date format.
fn sql_date(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.trim().split('-').collect();
    if let [y, m, d] = p[..]
        && y.len() == 4
    {
        let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
        let day = value::day_number(y, m, d);
        return (value::civil(day) == (y, m, d)).then_some(day);
    }
    value::parse_date(s)
}

fn sql_timestamp(s: &str) -> Option<num::Num> {
    match s.trim().split_once(' ') {
        Some((d, t)) => Some(value::make_ts(sql_date(d)?, &value::parse_time(t)?)),
        None => value::parse_timestamp(s),
    }
}

fn is_null(v: &Value) -> bool {
    v.is_empty()
}

/// SQL ordering of two non-NULL values: numerically when either side is a
/// number, date or time (text is converted), else case-sensitive text.
fn sql_cmp(a: &Value, b: &Value) -> Ordering {
    let typed = |v: &Value| !matches!(v, Value::Text(_) | Value::Json { .. } | Value::Container(_));
    let coerce = |v: &Value, like: &Value| -> Option<num::Num> {
        match (v, like) {
            (Value::Text(s), Value::Date(_)) => sql_date(s).map(num::int),
            (Value::Text(s), Value::Time(_)) => value::parse_time(s),
            (Value::Text(s), Value::Timestamp(_)) => sql_timestamp(s),
            _ => v.to_num(),
        }
    };
    if typed(a) || typed(b) {
        let (x, y) = (coerce(a, b), coerce(b, a));
        return x.cmp(&y);
    }
    a.to_text().cmp(&b.to_text())
}

/// `%` any run, `_` one character; case-sensitive.
fn like(s: &[char], p: &[char]) -> bool {
    // dp[i]: the pattern so far matches s[..i].
    let mut dp = vec![false; s.len() + 1];
    dp[0] = true;
    for &pc in p {
        let mut next = vec![false; s.len() + 1];
        for i in (0..=s.len()).filter(|&i| dp[i]) {
            match pc {
                '%' => next[i..].iter_mut().for_each(|x| *x = true),
                '_' if i < s.len() => next[i + 1] = true,
                c if s.get(i) == Some(&c) => next[i + 1] = true,
                _ => {}
            }
        }
        dp = next;
    }
    dp[s.len()]
}

/// How ExecuteSQL writes a value: ISO dates and times, plain numbers.
fn sql_text(v: &Value) -> String {
    let date = |d: i64| {
        let (y, m, dd) = civil(d);
        format!("{y:04}-{m:02}-{dd:02}")
    };
    let time = |t: &num::Num| {
        let s = value::format_time(t);
        // H:MM:SS -> HH:MM:SS
        if s.find(':') == Some(1) {
            format!("0{s}")
        } else {
            s
        }
    };
    match v {
        Value::Date(d) => date(*d),
        Value::Time(t) => time(t),
        Value::Timestamp(t) => format!("{} {}", date(value::ts_date(t)), time(&value::ts_time(t))),
        v => v.to_text(),
    }
}

/// Columns of the joined rows: (index of the table in FROM, name).
struct Schema<'a> {
    from: &'a [From],
    cols: Vec<(usize, String)>,
}

impl Schema<'_> {
    fn col(&self, qual: Option<&str>, name: &str) -> Result<usize, EvalError> {
        let hits: Vec<usize> = (0..self.cols.len())
            .filter(|&i| {
                let (t, c) = &self.cols[i];
                let t = &self.from[*t];
                c.eq_ignore_ascii_case(name)
                    && qual.is_none_or(|q| {
                        t.alias.eq_ignore_ascii_case(q) || t.name.eq_ignore_ascii_case(q)
                    })
            })
            .collect();
        match hits[..] {
            [i] => Ok(i),
            [] => Err(err(format!("column {name} does not exist"))),
            _ => Err(err(format!("column {name} is ambiguous"))),
        }
    }

    /// `e` with its columns resolved.
    fn bind(&self, e: &Ex) -> Result<Ex, EvalError> {
        match e {
            Ex::Col(q, n) => Ok(Ex::Idx(self.col(q.as_deref(), n)?)),
            _ => map(e, &mut |x| self.bind(x)),
        }
    }
}

type Mapper<'a> = dyn FnMut(&Ex) -> Result<Ex, EvalError> + 'a;

/// `e` with `f` applied to each sub-expression.
fn map(e: &Ex, f: &mut Mapper) -> Result<Ex, EvalError> {
    let mut b = |x: &Ex| f(x).map(Box::new);
    Ok(match e {
        Ex::Col(..) | Ex::Idx(_) | Ex::Lit(_) | Ex::Select(_) => e.clone(),
        Ex::Agg(a, x, d) => Ex::Agg(*a, x.as_deref().map(&mut b).transpose()?, *d),
        Ex::Cmp(op, x, y) => Ex::Cmp(op, b(x)?, b(y)?),
        Ex::Like(x, y, n) => Ex::Like(b(x)?, b(y)?, *n),
        Ex::In(x, v, n) => {
            let x = b(x)?;
            Ex::In(
                x,
                v.iter()
                    .map(|i| b(i).map(|i| *i))
                    .collect::<Result<_, _>>()?,
                *n,
            )
        }
        Ex::Between(x, l, h, n) => Ex::Between(b(x)?, b(l)?, b(h)?, *n),
        Ex::Null(x, n) => Ex::Null(b(x)?, *n),
        Ex::And(x, y) => Ex::And(b(x)?, b(y)?),
        Ex::Or(x, y) => Ex::Or(b(x)?, b(y)?),
        Ex::Not(x) => Ex::Not(b(x)?),
        Ex::Func(n, v) => Ex::Func(
            n.clone(),
            v.iter()
                .map(|i| b(i).map(|i| *i))
                .collect::<Result<_, _>>()?,
        ),
        Ex::Case(i, arms, o) => {
            let i = i.as_deref().map(&mut b).transpose()?;
            let arms = arms
                .iter()
                .map(|(w, t)| Ok((*b(w)?, *b(t)?)))
                .collect::<Result<_, EvalError>>()?;
            Ex::Case(i, arms, o.as_deref().map(&mut b).transpose()?)
        }
    })
}

/// `e` with its subqueries run and the clock and session functions read.
fn resolve(ev: &mut Evaluator, e: &Ex) -> Result<Ex, EvalError> {
    let first = |rows: Vec<Row>| rows.into_iter().filter_map(|r| r.into_iter().next());
    match e {
        Ex::Select(q) => Ok(Ex::Lit(
            first(select(ev, q)?).next().unwrap_or_else(Value::empty),
        )),
        Ex::In(x, items, n) => {
            let mut v = vec![];
            for i in items {
                match i {
                    Ex::Select(q) => v.extend(first(select(ev, q)?).map(Ex::Lit)),
                    i => v.push(resolve(ev, i)?),
                }
            }
            Ok(Ex::In(Box::new(resolve(ev, x)?), v, *n))
        }
        Ex::Func(n, a) if a.is_empty() && NOW.contains(&n.as_str()) => {
            let mut get = |f: &str| {
                ev.ctx
                    .get(f)
                    .or_else(|| super::builtin_get(f))
                    .unwrap_or_else(Value::empty)
            };
            Ok(Ex::Lit(match n.as_str() {
                "CURRENT_DATE" | "CURDATE" | "TODAY" => get("CurrentDate"),
                "CURRENT_TIME" | "CURTIME" => get("CurrentTime"),
                "CURRENT_TIMESTAMP" | "CURTIMESTAMP" => get("CurrentTimestamp"),
                "PI" => Value::Number(num::pi(15)),
                _ => get("AccountName"),
            }))
        }
        _ => map(e, &mut |x| resolve(ev, x)),
    }
}

/// Whether `e` has an aggregate; `cols` gets the columns outside them.
fn aggregates(e: &Ex, cols: &mut Vec<usize>) -> bool {
    match e {
        Ex::Agg(..) => true,
        Ex::Idx(i) => {
            cols.push(*i);
            false
        }
        Ex::Col(..) | Ex::Lit(_) => false,
        Ex::Cmp(_, x, y) | Ex::Like(x, y, _) | Ex::And(x, y) | Ex::Or(x, y) => {
            aggregates(x, cols) | aggregates(y, cols)
        }
        Ex::In(x, v, _) => v
            .iter()
            .fold(aggregates(x, cols), |a, i| aggregates(i, cols) | a),
        Ex::Between(x, l, h, _) => aggregates(x, cols) | aggregates(l, cols) | aggregates(h, cols),
        Ex::Null(x, _) | Ex::Not(x) => aggregates(x, cols),
        Ex::Func(_, v) => v.iter().fold(false, |a, i| aggregates(i, cols) | a),
        Ex::Case(i, arms, o) => i
            .as_deref()
            .into_iter()
            .chain(arms.iter().flat_map(|(w, t)| [w, t]))
            .chain(o.as_deref())
            .fold(false, |a, x| aggregates(x, cols) | a),
        Ex::Select(_) => false,
    }
}

type Row = Vec<Value>;

/// Value of `e` over a group of rows (one row outside GROUP BY); columns
/// read the group's first row.
fn value(e: &Ex, g: &[&Row]) -> Value {
    match e {
        Ex::Idx(i) => g
            .first()
            .and_then(|r| r.get(*i))
            .cloned()
            .unwrap_or_else(Value::empty),
        Ex::Lit(v) => v.clone(),
        Ex::Agg(f, a, distinct) => {
            let Some(a) = a else {
                return Value::int(g.len() as i64);
            };
            let mut vals: Vec<Value> = vec![];
            for r in g {
                let v = value(a, &[r]);
                if !is_null(&v) && !(*distinct && vals.iter().any(|x| sql_cmp(x, &v).is_eq())) {
                    vals.push(v);
                }
            }
            let nums = || vals.iter().filter_map(Value::to_num);
            let sum = || nums().fold(num::int(0), |a, n| num::add(&a, &n));
            match f {
                Agg::Count => Value::int(vals.len() as i64),
                _ if vals.is_empty() => Value::empty(),
                Agg::Sum => Value::Number(sum()),
                Agg::Avg => num::div(&sum(), &num::int(nums().count() as i64), num::DEFAULT_SCALE)
                    .map_or_else(Value::empty, Value::Number),
                Agg::Min | Agg::Max => {
                    let pick = if *f == Agg::Min {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                    vals.iter()
                        .skip(1)
                        .fold(&vals[0], |m, v| if sql_cmp(v, m) == pick { v } else { m })
                        .clone()
                }
            }
        }
        Ex::Func(n, args) => {
            let v: Vec<Value> = args.iter().map(|x| value(x, g)).collect();
            func(n, &v)
        }
        Ex::Case(input, arms, other) => {
            let input = input.as_ref().map(|i| value(i, g));
            for (w, then) in arms {
                let hit = match &input {
                    Some(x) => {
                        let y = value(w, g);
                        !is_null(x) && !is_null(&y) && sql_cmp(x, &y).is_eq()
                    }
                    None => test(w, g),
                };
                if hit {
                    return value(then, g);
                }
            }
            other.as_ref().map_or_else(Value::empty, |o| value(o, g))
        }
        Ex::Col(..) | Ex::Select(_) => Value::empty(),
        _ => Value::bool(test(e, g)),
    }
}

/// A number from SQL text (`'12.5'`), or any non-text value's number.
fn number(v: &Value) -> Option<num::Num> {
    match v {
        Value::Text(s) => num::parse(s.trim()),
        v => v.to_num(),
    }
}

fn date_of(v: &Value) -> Option<i64> {
    match v {
        Value::Date(d) => Some(*d),
        Value::Timestamp(t) => Some(value::ts_date(t)),
        Value::Text(s) => sql_date(s),
        _ => None,
    }
}

fn time_of(v: &Value) -> Option<num::Num> {
    match v {
        Value::Time(t) => Some(t.clone()),
        Value::Timestamp(t) => Some(value::ts_time(t)),
        Value::Text(s) => value::parse_time(s),
        _ => None,
    }
}

/// CAST ( v AS ty ); a value that does not convert is NULL.
fn cast(v: &Value, ty: &str) -> Option<Value> {
    Some(match ty {
        "VARCHAR" | "CHAR" | "CHARACTER" | "NVARCHAR" | "LONGVARCHAR" | "TEXT" => {
            Value::Text(sql_text(v))
        }
        "INT" | "INTEGER" | "SMALLINT" | "BIGINT" | "TINYINT" => {
            Value::Number(num::trunc(&number(v)?, 0))
        }
        "NUMERIC" | "DECIMAL" | "DOUBLE" | "FLOAT" | "REAL" | "NUMBER" => Value::Number(number(v)?),
        "DATE" => Value::Date(date_of(v)?),
        "TIME" => Value::Time(time_of(v)?),
        "TIMESTAMP" => Value::Timestamp(match v {
            Value::Timestamp(t) => t.clone(),
            Value::Date(d) => value::make_ts(*d, &num::int(0)),
            Value::Text(s) => sql_timestamp(s)?,
            _ => return None,
        }),
        _ => return None,
    })
}

/// `+ - * /` and unary minus. Text joins (`-` moves the left side's
/// trailing blanks to the end); a date plus or minus days is a date, and
/// two dates differ by a number of days.
fn arith(op: &str, a: &Value, b: &Value) -> Option<Value> {
    let text = |v: &Value| matches!(v, Value::Text(_) | Value::Json { .. });
    let days = |v: &Value| number(v).and_then(|n| n.to_i64());
    if op == "NEG" {
        return Some(Value::Number(-number(a)?));
    }
    if text(a) || text(b) {
        let (l, r) = (sql_text(a), sql_text(b));
        return match op {
            "+" => Some(Value::Text(l + &r)),
            "-" => {
                let t = l.trim_end_matches(' ');
                Some(Value::Text(format!("{t}{r}{}", &l[t.len()..])))
            }
            _ => arith(op, &Value::Number(number(a)?), &Value::Number(number(b)?)),
        };
    }
    match (op, a, b) {
        ("+", Value::Date(d), n) | ("+", n, Value::Date(d)) => {
            return Some(Value::Date(d + days(n)?));
        }
        ("-", Value::Date(x), Value::Date(y)) => return Some(Value::int(x - y)),
        ("-", Value::Date(d), n) => return Some(Value::Date(d - days(n)?)),
        _ => {}
    }
    let (x, y) = (a.to_num()?, b.to_num()?);
    Some(Value::Number(match op {
        "+" => num::add(&x, &y),
        "-" => num::sub(&x, &y),
        "*" => num::mul(&x, &y),
        _ => num::div(&x, &y, num::DEFAULT_SCALE)?,
    }))
}

/// A scalar function of [`FUNCS`] or an operator; NULL in, NULL out
/// (except COALESCE and NULLIF).
fn func(name: &str, v: &[Value]) -> Value {
    let arg = |i: usize| v.get(i).cloned().unwrap_or_else(Value::empty);
    match name {
        "COALESCE" => {
            return v
                .iter()
                .find(|x| !is_null(x))
                .cloned()
                .unwrap_or_else(Value::empty);
        }
        "NULLIF" => {
            let (a, b) = (arg(0), arg(1));
            let same = !is_null(&a) && !is_null(&b) && sql_cmp(&a, &b).is_eq();
            return if same { Value::empty() } else { a };
        }
        _ if v.iter().any(is_null) => return Value::empty(),
        _ => {}
    }
    scalar(name, v, &arg).unwrap_or_else(Value::empty)
}

fn scalar(name: &str, v: &[Value], arg: &dyn Fn(usize) -> Value) -> Option<Value> {
    let text = |i: usize| sql_text(&arg(i));
    let chars = |i: usize| text(i).chars().collect::<Vec<char>>();
    let n = |i: usize| number(&arg(i));
    let count = |i: usize| n(i).and_then(|x| num::trunc(&x, 0).to_i64());
    let on_num = |f: &dyn Fn(num::Num) -> Option<num::Num>| n(0).and_then(f).map(Value::Number);
    let int = |x: i64| Some(Value::int(x));
    let t = |s: String| Some(Value::Text(s));
    match name {
        "+" | "-" | "*" | "/" | "NEG" => arith(name, &arg(0), &arg(1)),
        "||" => t(text(0) + &text(1)),
        "UPPER" => t(text(0).to_uppercase()),
        "LOWER" => t(text(0).to_lowercase()),
        "TRIM" => t(text(0).trim_matches(' ').into()),
        "LTRIM" => t(text(0).trim_start_matches(' ').into()),
        "RTRIM" => t(text(0).trim_end_matches(' ').into()),
        "LEFT" => t(chars(0).iter().take(count(1)?.max(0) as usize).collect()),
        "RIGHT" => {
            let c = chars(0);
            let k = (count(1)?.max(0) as usize).min(c.len());
            t(c[c.len() - k..].iter().collect())
        }
        "SUBSTR" | "SUBSTRING" => {
            let len = match v.len() {
                2 => usize::MAX,
                _ => count(2)?.max(0) as usize,
            };
            let start = (count(1)?.max(1) - 1) as usize;
            t(chars(0).iter().skip(start).take(len).collect())
        }
        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" => int(chars(0).len() as i64),
        "POSITION" => {
            let (needle, hay) = (text(0), text(1));
            int(hay
                .find(&needle)
                .map_or(0, |b| hay[..b].chars().count() + 1) as i64)
        }
        "CAST" => cast(&arg(0), &text(1)),
        "ROUND" => {
            let places = if v.len() > 1 { count(1)? } else { 0 };
            on_num(&|x| Some(num::round(&x, places)))
        }
        "ABS" => on_num(&|x| Some(x.abs())),
        "SIGN" => on_num(&|x| Some(num::int(x.cmp(&num::int(0)) as i64))),
        "INT" => on_num(&|x| Some(num::trunc(&x, 0))),
        "FLOOR" => on_num(&|x| Some(num::floor(&x))),
        "CEIL" | "CEILING" => on_num(&|x| Some(num::ceil(&x))),
        "SQRT" => on_num(&|x| num::sqrt(&x, 15)),
        "MOD" => {
            let (x, y) = (n(0)?, n(1)?);
            let q = num::trunc(&num::div(&x, &y, num::DEFAULT_SCALE)?, 0);
            Some(Value::Number(num::sub(&x, &num::mul(&q, &y))))
        }
        "CHR" => t(char::from_u32(u32::try_from(count(0)?).ok()?)?.to_string()),
        "SPACE" => t(" ".repeat(count(0)?.clamp(0, 1_000_000) as usize)),
        "STRVAL" => t(text(0)),
        "NUMVAL" => n(0).map(Value::Number),
        "DATEVAL" => cast(&arg(0), "DATE"),
        "TIMESTAMPVAL" => cast(&arg(0), "TIMESTAMP"),
        "YEAR" | "MONTH" | "DAY" | "DAYOFWEEK" | "DAYNAME" | "MONTHNAME" => {
            let d = date_of(&arg(0))?;
            let (y, m, dd) = civil(d);
            let dow = value::day_of_week(d);
            match name {
                "YEAR" => int(y),
                "MONTH" => int(m),
                "DAY" => int(dd),
                "DAYOFWEEK" => int(dow),
                "DAYNAME" => t(super::datetime::DAYS[(dow - 1) as usize].into()),
                _ => t(super::datetime::MONTHS[(m - 1) as usize].into()),
            }
        }
        "HOUR" | "MINUTE" | "SECOND" => {
            let secs = num::floor(&time_of(&arg(0))?).to_i64()?;
            int(match name {
                "HOUR" => secs / 3600,
                "MINUTE" => secs / 60 % 60,
                _ => secs % 60,
            })
        }
        _ => None,
    }
}

fn test(e: &Ex, g: &[&Row]) -> bool {
    let both = |a: &Ex, b: &Ex| {
        let (x, y) = (value(a, g), value(b, g));
        (!is_null(&x) && !is_null(&y)).then(|| sql_cmp(&x, &y))
    };
    match e {
        Ex::And(a, b) => test(a, g) && test(b, g),
        Ex::Or(a, b) => test(a, g) || test(b, g),
        Ex::Not(a) => !test(a, g),
        Ex::Null(a, neg) => is_null(&value(a, g)) != *neg,
        Ex::Cmp(op, a, b) => both(a, b).is_some_and(|o| match *op {
            "=" => o.is_eq(),
            "<>" | "!=" => o.is_ne(),
            "<" => o.is_lt(),
            "<=" => o.is_le(),
            ">" => o.is_gt(),
            _ => o.is_ge(),
        }),
        Ex::Like(a, p, neg) => {
            let (s, p) = (value(a, g), value(p, g));
            !is_null(&s) && {
                let (s, p): (Vec<char>, Vec<char>) =
                    (s.to_text().chars().collect(), p.to_text().chars().collect());
                like(&s, &p) != *neg
            }
        }
        Ex::In(a, items, neg) => {
            let v = value(a, g);
            let found = items.iter().any(|i| {
                let x = value(i, g);
                !is_null(&x) && sql_cmp(&v, &x).is_eq()
            });
            !is_null(&v) && found != *neg
        }
        Ex::Between(a, lo, hi, neg) => {
            let inside = both(a, lo).is_some_and(Ordering::is_ge)
                && both(a, hi).is_some_and(Ordering::is_le);
            inside != *neg
        }
        Ex::Col(..)
        | Ex::Idx(_)
        | Ex::Lit(_)
        | Ex::Agg(..)
        | Ex::Func(..)
        | Ex::Case(..)
        | Ex::Select(_) => value(e, g).truth(),
    }
}

/// SQL ordering with NULL first.
fn null_first(x: &Value, y: &Value) -> Ordering {
    match (is_null(x), is_null(y)) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => sql_cmp(x, y),
    }
}

/// The rows of a FROM table: a system table, else the host's.
fn table_rows(ev: &mut Evaluator, name: &str) -> Result<Rows, EvalError> {
    let up = name.to_ascii_uppercase();
    let system = [
        "FILEMAKER_TABLES",
        "FILEMAKER_FIELDS",
        "FILEMAKER_BASETABLES",
        "FILEMAKER_BASETABLEFIELDS",
    ];
    let (true, Some(s)) = (system.contains(&up.as_str()), ev.ctx.schema("")) else {
        return ev.ctx.rows(name);
    };
    let file = ev
        .ctx
        .get("FileName")
        .map(|v| v.to_text())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| s.file.clone());
    let cols = |c: &[&str]| c.iter().map(|c| c.to_string()).collect();
    let text = |s: &str| Value::text(s);
    let id = |n: u64| Value::int(n as i64);
    // TableName or BaseTableName, then the field's columns.
    let field_row = |table: &str, f: &crate::schema::Field| {
        use crate::schema::{DataType as D, Storage as S};
        let ty = match f.data_type {
            D::Text => "varchar",
            D::Number => "decimal",
            D::Date => "date",
            D::Time => "time",
            D::Timestamp => "timestamp",
            D::Container => "binary",
        };
        let ty = if f.storage == S::Global {
            format!("global {ty}")
        } else {
            ty.to_string()
        };
        let class = match f.storage {
            S::Summary => "Summary",
            S::StoredCalc | S::UnstoredCalc => "Calculated",
            _ => "Normal",
        };
        vec![
            text(table),
            text(&f.name),
            Value::Text(ty),
            id(f.id),
            text(class),
            Value::int(f.repetitions.max(1) as i64),
            Value::empty(),
        ]
    };
    let base = |name: &str| s.base_tables.iter().find(|b| b.name == name);
    let fields = [
        "FieldName",
        "FieldType",
        "FieldId",
        "FieldClass",
        "FieldReps",
        "ModCount",
    ];
    Ok(match up.as_str() {
        "FILEMAKER_TABLES" => Rows {
            columns: cols(&[
                "TableName",
                "TableId",
                "BaseTableName",
                "BaseFileName",
                "ModCount",
            ]),
            rows: s
                .occurrences
                .iter()
                .map(|o| {
                    let f = if o.source.is_empty() {
                        &file
                    } else {
                        &o.source
                    };
                    vec![
                        text(&o.name),
                        id(o.id),
                        text(&o.base_table),
                        text(f),
                        Value::empty(),
                    ]
                })
                .collect(),
        },
        "FILEMAKER_FIELDS" => Rows {
            columns: cols(&[&["TableName"][..], &fields].concat()),
            rows: s
                .occurrences
                .iter()
                .flat_map(|o| {
                    base(&o.base_table)
                        .into_iter()
                        .flat_map(|b| b.fields.iter().map(|f| field_row(&o.name, f)))
                })
                .collect(),
        },
        "FILEMAKER_BASETABLES" => Rows {
            columns: cols(&["BaseTableName", "BaseTableId", "Source", "ModCount"]),
            rows: s
                .base_tables
                .iter()
                .map(|b| vec![text(&b.name), id(b.id), text("<Internal>"), Value::empty()])
                .collect(),
        },
        _ => Rows {
            columns: cols(&[&["BaseTableName"][..], &fields].concat()),
            rows: s
                .base_tables
                .iter()
                .flat_map(|b| b.fields.iter().map(|f| field_row(&b.name, f)))
                .collect(),
        },
    })
}

/// `q` with its subqueries run and its clock functions read.
fn resolve_query(ev: &mut Evaluator, q: &Query) -> Result<Query, EvalError> {
    let mut q = q.clone();
    let mut r = |e: &mut Ex| -> Result<(), EvalError> {
        *e = resolve(ev, e)?;
        Ok(())
    };
    for i in &mut q.items {
        if let Item::Ex(e, _) = i {
            r(e)?;
        }
    }
    for f in &mut q.from {
        if let Some(e) = &mut f.on {
            r(e)?;
        }
    }
    for e in q
        .filter
        .iter_mut()
        .chain(q.having.iter_mut())
        .chain(q.group.iter_mut())
        .chain(q.order.iter_mut().map(|(e, _)| e))
    {
        r(e)?;
    }
    Ok(q)
}

fn run(ev: &mut Evaluator, a: &[Value]) -> Result<String, EvalError> {
    let mut p = P {
        t: lex(&txt(a, 0))?,
        i: 0,
        params: &a[3..],
        next_param: 0,
    };
    let q = p.query()?;
    if p.i < p.t.len() {
        return Err(err("unexpected text after the query"));
    }
    let fs = match txt(a, 1) {
        s if s.is_empty() => ",".to_string(),
        s => s,
    };
    let rs = match txt(a, 2) {
        s if s.is_empty() => "\r".to_string(),
        s => s,
    };
    Ok(select(ev, &q)?
        .iter()
        .map(|r| r.iter().map(sql_text).collect::<Vec<_>>().join(&fs))
        .collect::<Vec<_>>()
        .join(&rs))
}

/// The rows a query selects, in order.
fn select(ev: &mut Evaluator, q: &Query) -> Result<Vec<Row>, EvalError> {
    let q = &resolve_query(ev, q)?;
    let mut tables = vec![];
    for f in &q.from {
        let mut rows = table_rows(ev, &f.name)?;
        // Short rows from the host read as NULL.
        let width = rows.columns.len();
        rows.rows
            .iter_mut()
            .for_each(|r| r.resize(width, Value::empty()));
        tables.push(rows);
    }
    let schema = Schema {
        from: &q.from,
        cols: tables
            .iter()
            .enumerate()
            .flat_map(|(t, r)| r.columns.iter().map(move |c| (t, c.clone())))
            .collect(),
    };
    // Resolve every column first: a bad one is an error even with no rows.
    let bind_opt = |e: &Option<Ex>| e.as_ref().map(|e| schema.bind(e)).transpose();
    let ons = q
        .from
        .iter()
        .map(|f| bind_opt(&f.on))
        .collect::<Result<Vec<_>, _>>()?;
    let filter = bind_opt(&q.filter)?;
    let having = bind_opt(&q.having)?;
    let group = q
        .group
        .iter()
        .map(|e| schema.bind(e))
        .collect::<Result<Vec<_>, _>>()?;
    // The output columns, `*` expanded, and their names (AS).
    let mut outs: Vec<(Ex, Option<String>)> = vec![];
    for i in &q.items {
        match i {
            Item::All(t) => {
                let t = match t {
                    Some(t) => Some(
                        q.from
                            .iter()
                            .position(|f| {
                                f.alias.eq_ignore_ascii_case(t) || f.name.eq_ignore_ascii_case(t)
                            })
                            .ok_or_else(|| err(format!("table {t} is not in FROM")))?,
                    ),
                    None => None,
                };
                for (c, (ct, _)) in schema.cols.iter().enumerate() {
                    if t.is_none_or(|t| t == *ct) {
                        outs.push((Ex::Idx(c), None));
                    }
                }
            }
            Item::Ex(e, alias) => outs.push((schema.bind(e)?, alias.clone())),
        }
    }
    let mut order = vec![];
    for (k, desc) in &q.order {
        let alias = match k {
            Ex::Col(None, n) => outs
                .iter()
                .find(|(_, a)| a.as_ref().is_some_and(|a| a.eq_ignore_ascii_case(n))),
            _ => None,
        };
        let key = match (k, alias) {
            (_, Some((e, _))) => e.clone(),
            (Ex::Lit(n), _) => {
                let i = crate::eval::to_index(n)? as usize;
                outs.get(i.wrapping_sub(1))
                    .ok_or_else(|| err("invalid ORDER BY position"))?
                    .0
                    .clone()
            }
            _ => schema.bind(k)?,
        };
        order.push((key, *desc));
    }
    // Aggregates group the rows; other columns must be GROUP BY columns.
    let mut loose = vec![];
    let mut grouped = !group.is_empty();
    for e in outs
        .iter()
        .map(|(e, _)| e)
        .chain(&having)
        .chain(order.iter().map(|(e, _)| e))
    {
        grouped |= aggregates(e, &mut loose);
    }
    if grouped
        && loose
            .iter()
            .any(|c| !group.iter().any(|g| matches!(g, Ex::Idx(i) if i == c)))
    {
        return Err(err("a selected column is not in GROUP BY"));
    }
    // FROM and JOIN: the rows of each table joined to those before it.
    let mut rows: Vec<Row> = vec![vec![]];
    for ((t, f), on) in tables.iter().zip(&q.from).zip(&ons) {
        let mut next = vec![];
        for l in &rows {
            let mut hit = false;
            for r in &t.rows {
                let mut row = l.clone();
                row.extend(r.iter().cloned());
                if on.as_ref().is_none_or(|on| test(on, &[&row])) {
                    hit = true;
                    next.push(row);
                }
            }
            if f.left && !hit {
                let mut row = l.clone();
                row.resize(l.len() + t.columns.len(), Value::empty());
                next.push(row);
            }
        }
        rows = next;
    }
    let hits: Vec<&Row> = rows
        .iter()
        .filter(|r| filter.as_ref().is_none_or(|f| test(f, &[r])))
        .collect();
    let mut groups: Vec<Vec<&Row>> = if group.is_empty() {
        if grouped {
            vec![hits]
        } else {
            hits.into_iter().map(|r| vec![r]).collect()
        }
    } else {
        let mut at: std::collections::HashMap<Vec<String>, usize> = Default::default();
        let mut gs: Vec<Vec<&Row>> = vec![];
        for r in hits {
            let key: Vec<String> = group.iter().map(|g| sql_text(&value(g, &[r]))).collect();
            let i = *at.entry(key).or_insert_with(|| {
                gs.push(vec![]);
                gs.len() - 1
            });
            gs[i].push(r);
        }
        gs
    };
    if let Some(h) = &having {
        groups.retain(|g| test(h, g));
    }
    let keys: Vec<Vec<Value>> = groups
        .iter()
        .map(|g| order.iter().map(|(k, _)| value(k, g)).collect())
        .collect();
    let mut idx: Vec<usize> = (0..groups.len()).collect();
    idx.sort_by(|&x, &y| {
        for (n, (_, desc)) in order.iter().enumerate() {
            let o = null_first(&keys[x][n], &keys[y][n]);
            let o = if *desc { o.reverse() } else { o };
            if o.is_ne() {
                return o;
            }
        }
        Ordering::Equal
    });
    let mut out: Vec<Row> = vec![];
    let mut seen = std::collections::HashSet::new();
    for i in idx {
        let row: Row = outs.iter().map(|(e, _)| value(e, &groups[i])).collect();
        if !q.distinct || seen.insert(row.iter().map(sql_text).collect::<Vec<_>>()) {
            out.push(row);
        }
    }
    Ok(out
        .into_iter()
        .skip(q.offset)
        .take(q.limit.unwrap_or(usize::MAX))
        .collect())
}

/// ExecuteSQL ( query ; fieldSeparator ; rowSeparator { ; arguments... } ).
/// Any error gives `?`.
pub fn execute_sql(ev: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(run(ev, a).unwrap_or_else(|_| "?".into())))
}

/// ExecuteSQLe: like ExecuteSQL, but an error gives `? ` and its message.
// ponytail: FileMaker's exact error wording is not reproduced.
pub fn execute_sql_e(ev: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(
        run(ev, a).unwrap_or_else(|e| format!("? {}", e.message)),
    ))
}

#[cfg(test)]
mod tests {
    use super::like;

    #[test]
    fn like_patterns() {
        let l = |s: &str, p: &str| {
            like(
                &s.chars().collect::<Vec<_>>(),
                &p.chars().collect::<Vec<_>>(),
            )
        };
        assert!(l("Smith", "S%"));
        assert!(l("Smith", "%ith"));
        assert!(l("Smith", "Sm_th"));
        assert!(!l("Smith", "s%"));
        assert!(!l("Smith", "Sm_h"));
    }
}
