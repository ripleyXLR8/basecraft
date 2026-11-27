//! ExecuteSQL: a minimal SELECT over the rows a host returns from
//! [`Context::rows`](crate::Context::rows).
//!
//! Supported: `SELECT [DISTINCT] * | col, ... | COUNT(*) FROM t [alias]
//! [WHERE ...] [ORDER BY col|n [ASC|DESC], ...] [OFFSET n ROWS]
//! [FETCH FIRST n ROWS ONLY]`, with `= <> != < <= > >=`, `LIKE`, `IN`,
//! `BETWEEN`, `IS [NOT] NULL`, `AND`, `OR`, `NOT`, `?` parameters and
//! `DATE 'yyyy-mm-dd'` / `TIME` / `TIMESTAMP` literals. Text comparisons are
//! case-sensitive and an empty value is NULL, as in FileMaker's SQL.
// ponytail: no joins, GROUP BY, expressions in the select list or the
// FileMaker_Tables / FileMaker_Fields system tables; add when files use them.

use super::txt;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num;
use crate::schema::Rows;
use crate::value::{self, Value, civil};
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
            let sym = ["<>", "!=", "<=", ">="]
                .into_iter()
                .find(|s| *s == two)
                .or_else(|| {
                    ["=", "<", ">", "(", ")", ",", "*", "."]
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

#[derive(Debug)]
enum Ex {
    Col(String),
    Lit(Value),
    Cmp(&'static str, Box<Ex>, Box<Ex>),
    Like(Box<Ex>, Box<Ex>, bool),
    In(Box<Ex>, Vec<Ex>, bool),
    Between(Box<Ex>, Box<Ex>, Box<Ex>, bool),
    Null(Box<Ex>, bool),
    And(Box<Ex>, Box<Ex>),
    Or(Box<Ex>, Box<Ex>),
    Not(Box<Ex>),
}

enum Item {
    All,
    Count,
    Col(String),
}

struct Query {
    distinct: bool,
    items: Vec<Item>,
    table: String,
    filter: Option<Ex>,
    order: Vec<(Ex, bool)>,
    offset: usize,
    limit: Option<usize>,
}

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

    fn kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(k)) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn sym(&mut self, s: &str) -> bool {
        if self.peek()
            == Some(&Tok::Sym(match s {
                "(" => "(",
                ")" => ")",
                "," => ",",
                "*" => "*",
                "." => ".",
                _ => return false,
            }))
        {
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

    /// `col` or `table.col`; the qualifier is dropped (one table only).
    fn column(&mut self) -> Result<String, EvalError> {
        let n = self.name()?;
        if self.sym(".") {
            return self.name();
        }
        Ok(n)
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

    fn query(&mut self) -> Result<Query, EvalError> {
        self.expect_kw("SELECT")?;
        let distinct = self.kw("DISTINCT");
        let mut items = vec![];
        loop {
            if self.sym("*") {
                items.push(Item::All);
            } else if matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("COUNT"))
                && self.t.get(self.i + 1) == Some(&Tok::Sym("("))
            {
                self.i += 2;
                self.expect("*")?;
                self.expect(")")?;
                items.push(Item::Count);
            } else {
                items.push(Item::Col(self.column()?));
            }
            if !self.sym(",") {
                break;
            }
        }
        self.expect_kw("FROM")?;
        let table = self.name()?;
        // optional alias
        if let Some(Tok::Word(w)) = self.peek()
            && !["WHERE", "ORDER", "OFFSET", "FETCH"]
                .iter()
                .any(|k| w.eq_ignore_ascii_case(k))
        {
            self.i += 1;
        }
        let filter = if self.kw("WHERE") {
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
                    _ => Ex::Col(self.column()?),
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
        if self.i < self.t.len() {
            return Err(err("unexpected text after the query"));
        }
        Ok(Query {
            distinct,
            items,
            table,
            filter,
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
        let l = self.operand()?;
        if let Some(Tok::Sym(s)) = self.peek().cloned()
            && ["=", "<>", "!=", "<", "<=", ">", ">="].contains(&s)
        {
            self.i += 1;
            let r = self.operand()?;
            return Ok(Ex::Cmp(s, Box::new(l), Box::new(r)));
        }
        if self.kw("IS") {
            let neg = self.kw("NOT");
            self.expect_kw("NULL")?;
            return Ok(Ex::Null(Box::new(l), neg));
        }
        let neg = self.kw("NOT");
        if self.kw("LIKE") {
            return Ok(Ex::Like(Box::new(l), Box::new(self.operand()?), neg));
        }
        if self.kw("IN") {
            self.expect("(")?;
            let mut items = vec![self.operand()?];
            while self.sym(",") {
                items.push(self.operand()?);
            }
            self.expect(")")?;
            return Ok(Ex::In(Box::new(l), items, neg));
        }
        if self.kw("BETWEEN") {
            let lo = self.operand()?;
            self.expect_kw("AND")?;
            let hi = self.operand()?;
            return Ok(Ex::Between(Box::new(l), Box::new(lo), Box::new(hi), neg));
        }
        if neg {
            return Err(err("LIKE, IN or BETWEEN expected after NOT"));
        }
        Ok(l)
    }

    fn operand(&mut self) -> Result<Ex, EvalError> {
        if self.sym("(") {
            let e = self.or()?;
            self.expect(")")?;
            return Ok(e);
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
            Tok::Word(_) | Tok::Quoted(_) => Ok(Ex::Col(self.column()?)),
            Tok::Sym(s) => Err(err(format!("unexpected {s}"))),
        }
    }
}

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

struct Table<'a> {
    rows: &'a Rows,
}

impl Table<'_> {
    fn col(&self, name: &str) -> Result<usize, EvalError> {
        self.rows
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .ok_or_else(|| err(format!("column {name} does not exist")))
    }

    fn value(&self, e: &Ex, row: &[Value]) -> Result<Value, EvalError> {
        Ok(match e {
            Ex::Col(c) => row[self.col(c)?].clone(),
            Ex::Lit(v) => v.clone(),
            _ => Value::bool(self.test(e, row)?),
        })
    }

    fn test(&self, e: &Ex, row: &[Value]) -> Result<bool, EvalError> {
        let both = |a: &Ex, b: &Ex| -> Result<Option<Ordering>, EvalError> {
            let (x, y) = (self.value(a, row)?, self.value(b, row)?);
            Ok((!is_null(&x) && !is_null(&y)).then(|| sql_cmp(&x, &y)))
        };
        Ok(match e {
            Ex::And(a, b) => self.test(a, row)? && self.test(b, row)?,
            Ex::Or(a, b) => self.test(a, row)? || self.test(b, row)?,
            Ex::Not(a) => !self.test(a, row)?,
            Ex::Null(a, neg) => is_null(&self.value(a, row)?) != *neg,
            Ex::Cmp(op, a, b) => both(a, b)?.is_some_and(|o| match *op {
                "=" => o.is_eq(),
                "<>" | "!=" => o.is_ne(),
                "<" => o.is_lt(),
                "<=" => o.is_le(),
                ">" => o.is_gt(),
                _ => o.is_ge(),
            }),
            Ex::Like(a, p, neg) => {
                let (s, p) = (self.value(a, row)?, self.value(p, row)?);
                !is_null(&s) && {
                    let (s, p): (Vec<char>, Vec<char>) =
                        (s.to_text().chars().collect(), p.to_text().chars().collect());
                    like(&s, &p) != *neg
                }
            }
            Ex::In(a, items, neg) => {
                let v = self.value(a, row)?;
                let mut found = false;
                for i in items {
                    let x = self.value(i, row)?;
                    found |= !is_null(&v) && !is_null(&x) && sql_cmp(&v, &x).is_eq();
                }
                !is_null(&v) && found != *neg
            }
            Ex::Between(a, lo, hi, neg) => {
                let inside = both(a, lo)?.is_some_and(Ordering::is_ge)
                    && both(a, hi)?.is_some_and(Ordering::is_le);
                inside != *neg
            }
            Ex::Col(_) | Ex::Lit(_) => self.value(e, row)?.truth(),
        })
    }
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

fn run(ev: &mut Evaluator, a: &[Value]) -> Result<String, EvalError> {
    let mut p = P {
        t: lex(&txt(a, 0))?,
        i: 0,
        params: &a[3..],
        next_param: 0,
    };
    let q = p.query()?;
    let mut rows = ev.ctx.rows(&q.table)?;
    // Short rows from the host read as NULL.
    let width = rows.columns.len();
    rows.rows
        .iter_mut()
        .for_each(|r| r.resize(width, Value::empty()));
    let t = Table { rows: &rows };
    let mut hits: Vec<&Vec<Value>> = vec![];
    for r in &rows.rows {
        if q.filter.as_ref().map_or(Ok(true), |f| t.test(f, r))? {
            hits.push(r);
        }
    }
    // Resolve ORDER BY keys first so a bad column is an error even with no rows.
    let mut keys = vec![];
    for (k, desc) in &q.order {
        let col = match k {
            Ex::Lit(n) => {
                let i = crate::eval::to_index(n)? as usize;
                match q.items.get(i.wrapping_sub(1)) {
                    Some(Item::Col(c)) => t.col(c)?,
                    _ if matches!(q.items[..], [Item::All])
                        && (1..=rows.columns.len()).contains(&i) =>
                    {
                        i - 1
                    }
                    _ => return Err(err("invalid ORDER BY position")),
                }
            }
            Ex::Col(c) => t.col(c)?,
            _ => unreachable!("ORDER BY keys are columns or positions"),
        };
        keys.push((col, *desc));
    }
    hits.sort_by(|x, y| {
        for &(c, desc) in &keys {
            let o = match (is_null(&x[c]), is_null(&y[c])) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => sql_cmp(&x[c], &y[c]),
            };
            let o = if desc { o.reverse() } else { o };
            if o.is_ne() {
                return o;
            }
        }
        Ordering::Equal
    });
    let fs = match txt(a, 1) {
        s if s.is_empty() => ",".to_string(),
        s => s,
    };
    let rs = match txt(a, 2) {
        s if s.is_empty() => "\r".to_string(),
        s => s,
    };
    if q.items.iter().any(|i| matches!(i, Item::Count)) {
        if q.items.len() > 1 {
            return Err(err(
                "COUNT(*) cannot be mixed with columns without GROUP BY",
            ));
        }
        return Ok(hits
            .len()
            .saturating_sub(q.offset)
            .min(q.limit.unwrap_or(usize::MAX))
            .to_string());
    }
    let mut cols = vec![];
    for i in &q.items {
        match i {
            Item::All => cols.extend(0..rows.columns.len()),
            Item::Col(c) => cols.push(t.col(c)?),
            Item::Count => unreachable!(),
        }
    }
    let mut out: Vec<String> = vec![];
    for r in hits {
        let line = cols
            .iter()
            .map(|&c| sql_text(&r[c]))
            .collect::<Vec<_>>()
            .join(&fs);
        if !q.distinct || !out.contains(&line) {
            out.push(line);
        }
    }
    let out: Vec<String> = out
        .into_iter()
        .skip(q.offset)
        .take(q.limit.unwrap_or(usize::MAX))
        .collect();
    Ok(out.join(&rs))
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
