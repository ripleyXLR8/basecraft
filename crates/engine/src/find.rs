//! Find mode: FileMaker find requests and criteria syntax.
//!
//! A find runs its requests in order, as FileMaker's: a find request adds
//! its matches to the set, an omit request removes its matches from it.
//! The set starts empty, or with every record when the first request is an
//! omit request. Requests without criteria are skipped (all of them empty
//! is FileMaker's error 400, for the caller). Within a request, every
//! criterion must match (AND).

use calc::Value;
use fmp12::schema::DataType;

use crate::{Database, typed};

#[derive(Debug, Clone, Default)]
pub struct Request {
    /// (field id, criterion text as typed in Find mode)
    pub criteria: Vec<(u32, String)>,
    pub omit: bool,
}

/// Runs `requests` over every record of `table`; returns the found set.
pub fn find(db: &Database, table: u32, requests: &[Request]) -> Vec<usize> {
    let Some(t) = db.table(table) else {
        return Vec::new();
    };
    let matches = |r: usize, req: &Request| {
        req.criteria.iter().all(|(f, crit)| {
            let dt = t.field(*f).map_or(DataType::Text, |f| f.data_type);
            matches_criterion(&db.value(table, r, *f), crit, dt)
        })
    };
    let mut reqs = requests
        .iter()
        .filter(|q| !q.criteria.is_empty())
        .peekable();
    let mut set = vec![reqs.peek().is_some_and(|q| q.omit); t.records.len()];
    for q in reqs {
        for (r, in_set) in set.iter_mut().enumerate() {
            if *in_set == q.omit && matches(r, q) {
                *in_set = !q.omit;
            }
        }
    }
    (0..set.len()).filter(|&r| set[r]).collect()
}

fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// Words of a value, as FileMaker indexes them.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '.' || c == '@' || c == '_'))
        .filter(|w| !w.is_empty())
        .map(fold)
        .collect()
}

/// `*` any run, `@` one character, `#` one digit.
fn glob(pat: &[char], s: &[char]) -> bool {
    match pat.split_first() {
        None => s.is_empty(),
        Some(('*', rest)) => (0..=s.len()).any(|i| glob(rest, &s[i..])),
        Some((p, rest)) => {
            let Some((c, srest)) = s.split_first() else {
                return false;
            };
            let ok = match p {
                '@' => true,
                '#' => c.is_ascii_digit(),
                _ => p.to_lowercase().eq(c.to_lowercase()),
            };
            ok && glob(rest, srest)
        }
    }
}

fn cmp_typed(v: &Value, crit: &str, dt: DataType) -> Option<std::cmp::Ordering> {
    let c = typed(crit.trim(), dt);
    if c.is_empty() {
        return None;
    }
    Some(calc::eval::compare(v, &c))
}

pub fn matches_criterion(v: &Value, crit: &str, dt: DataType) -> bool {
    use std::cmp::Ordering::*;
    let crit = crit.trim();
    let text = v.to_text();
    if crit.is_empty() {
        return true;
    }
    // Exact: "==" whole field, "=" alone = empty field, "=x" whole word.
    if let Some(rest) = crit.strip_prefix("==") {
        return glob(
            &fold(rest).chars().collect::<Vec<_>>(),
            &fold(&text).chars().collect::<Vec<_>>(),
        );
    }
    if crit == "=" {
        return text.is_empty();
    }
    if crit == "*" {
        return !text.is_empty();
    }
    for (op, want) in [
        ("<=", [Less, Equal]),
        (">=", [Greater, Equal]),
        ("≤", [Less, Equal]),
        ("≥", [Greater, Equal]),
    ] {
        if let Some(rest) = crit.strip_prefix(op) {
            return !text.is_empty() && cmp_typed(v, rest, dt).is_some_and(|o| want.contains(&o));
        }
    }
    if let Some(rest) = crit.strip_prefix('<') {
        return !text.is_empty() && cmp_typed(v, rest, dt) == Some(Less);
    }
    if let Some(rest) = crit.strip_prefix('>') {
        return !text.is_empty() && cmp_typed(v, rest, dt) == Some(Greater);
    }
    // Range a...b (also "a..b").
    if let Some((a, b)) = crit.split_once("...").or_else(|| crit.split_once("..")) {
        return !text.is_empty()
            && cmp_typed(v, a, dt).is_none_or(|o| o != Less)
            && cmp_typed(v, b, dt).is_none_or(|o| o != Greater);
    }
    if let Some(rest) = crit.strip_prefix('=') {
        return words(&text).contains(&fold(rest.trim()));
    }
    // Quoted phrase: substring.
    if crit.len() >= 2 && crit.starts_with('"') && crit.ends_with('"') {
        return fold(&text).contains(&fold(&crit[1..crit.len() - 1]));
    }
    match dt {
        DataType::Number | DataType::Date | DataType::Time | DataType::Timestamp => {
            cmp_typed(v, crit, dt) == Some(Equal)
        }
        _ => {
            // Every word of the criterion must start a word of the value.
            let vw = words(&text);
            crit.split_whitespace().all(|cw| {
                let p: Vec<char> = fold(cw).chars().chain(['*']).collect();
                vw.iter().any(|w| glob(&p, &w.chars().collect::<Vec<_>>()))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: &str, c: &str) -> bool {
        matches_criterion(&Value::text(v), c, DataType::Text)
    }

    #[test]
    fn text_criteria() {
        assert!(t("John Smith", "jo"));
        assert!(t("John Smith", "smi"));
        assert!(!t("John Smith", "ohn"));
        assert!(t("John Smith", "john sm"));
        assert!(t("", "="));
        assert!(!t("x", "="));
        assert!(t("x", "*"));
        assert!(t("John Smith", "==john smith"));
        assert!(!t("John Smith", "==john"));
        assert!(t("John Smith", "=john"));
        assert!(!t("Johnny", "=john"));
        assert!(t("abc", "a@c"));
        assert!(t("a1c", "a#c"));
        assert!(t("Hello world", "\"lo wo\""));
    }

    #[test]
    fn number_criteria() {
        let n =
            |v: &str, c: &str| matches_criterion(&typed(v, DataType::Number), c, DataType::Number);
        assert!(n("12", "12"));
        assert!(!n("12", "1"));
        assert!(n("12", ">10"));
        assert!(n("12", "<=12"));
        assert!(n("12", "10...20"));
        assert!(!n("25", "10..20"));
        assert!(!n("", "<5"));
    }
}
