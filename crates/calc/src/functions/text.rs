//! Text, value-list and word functions. Positions count Unicode scalar
//! values, 1-based.

use super::{int, txt};
use crate::ast::Expr;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R, compare};
use crate::value::Value;

fn chars(a: &[Value], i: usize) -> Vec<char> {
    txt(a, i).chars().collect()
}

fn t(c: &[char]) -> R {
    Ok(Value::Text(c.iter().collect()))
}

/// Character-for-character lower case, so indexes stay aligned.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn clamp_len(n: i64, len: usize) -> usize {
    n.clamp(0, len as i64) as usize
}

pub fn left(_: &mut Evaluator, a: &[Value]) -> R {
    let s = chars(a, 0);
    t(&s[..clamp_len(int(a, 1)?, s.len())])
}

pub fn right(_: &mut Evaluator, a: &[Value]) -> R {
    let s = chars(a, 0);
    t(&s[s.len() - clamp_len(int(a, 1)?, s.len())..])
}

pub fn middle(_: &mut Evaluator, a: &[Value]) -> R {
    let s = chars(a, 0);
    let start = clamp_len(int(a, 1)?.max(1) - 1, s.len());
    let n = clamp_len(int(a, 2)?, s.len() - start);
    t(&s[start..start + n])
}

pub fn length(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(txt(a, 0).chars().count() as i64))
}

/// 0-based start indexes of every (overlapping) case-insensitive match.
fn matches(hay: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return vec![];
    }
    let hay: Vec<char> = hay.iter().map(|&c| fold(c)).collect();
    let needle: Vec<char> = needle.iter().map(|&c| fold(c)).collect();
    (0..=hay.len() - needle.len())
        .filter(|&i| hay[i..i + needle.len()] == needle[..])
        .collect()
}

pub fn position(_: &mut Evaluator, a: &[Value]) -> R {
    let (s, needle) = (chars(a, 0), chars(a, 1));
    let (start, occ) = (int(a, 2)?, int(a, 3)?);
    let m = matches(&s, &needle);
    let found = if occ > 0 {
        let from = (start.max(1) - 1) as usize;
        m.iter()
            .filter(|&&i| i >= from)
            .nth(occ.unsigned_abs() as usize - 1)
    } else if occ < 0 {
        let from = start.min(s.len() as i64) - 1;
        m.iter()
            .rev()
            .filter(|&&i| i as i64 <= from)
            .nth(occ.unsigned_abs() as usize - 1)
    } else {
        None
    };
    Ok(Value::int(found.map_or(0, |&i| i as i64 + 1)))
}

pub fn pattern_count(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(matches(&chars(a, 0), &chars(a, 1)).len() as i64))
}

/// Substitute ( text ; search ; replace ) or with `[search ; replace]` lists,
/// applied one after another. Case-sensitive.
pub fn substitute(ev: &mut Evaluator, a: &[Expr]) -> R {
    let mut s = ev.eval(&a[0])?.to_text();
    let pairs: Vec<(&Expr, &Expr)> = if matches!(a[1], Expr::List(_)) {
        a[1..]
            .iter()
            .map(|e| match e {
                Expr::List(p) if p.len() == 2 => Ok((&p[0], &p[1])),
                _ => Err(EvalError::new(
                    code::TOO_FEW_PARAMS,
                    "Substitute lists need [search ; replace]",
                )),
            })
            .collect::<Result<_, _>>()?
    } else if a.len() == 3 {
        vec![(&a[1], &a[2])]
    } else {
        return Err(EvalError::new(
            code::TOO_FEW_PARAMS,
            "Substitute needs search and replace",
        ));
    };
    for (from, to) in pairs {
        let (from, to) = (ev.eval(from)?.to_text(), ev.eval(to)?.to_text());
        if !from.is_empty() {
            s = s.replace(&from, &to);
        }
    }
    Ok(Value::Text(s))
}

pub fn trim(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::text(txt(a, 0).trim_matches(' ')))
}

/// Roman-text behaviour only: trims the ends and collapses runs of spaces.
// ponytail: trimType's Japanese/Roman boundary rules are not modelled.
pub fn trim_all(_: &mut Evaluator, a: &[Value]) -> R {
    let full_width = int(a, 1)? != 0;
    let is_space = |c: char| c == ' ' || (full_width && c == '\u{3000}');
    let s = txt(a, 0);
    let mut out = String::new();
    let mut pending = false;
    for c in s.chars() {
        if is_space(c) {
            pending = !out.is_empty();
        } else {
            if pending {
                out.push(' ');
                pending = false;
            }
            out.push(c);
        }
    }
    Ok(Value::Text(out))
}

fn map_chars(a: &[Value], f: impl FnMut(char) -> char) -> R {
    Ok(Value::Text(txt(a, 0).chars().map(f).collect()))
}

fn single<I: Iterator<Item = char>>(c: char, mut it: I) -> char {
    match (it.next(), it.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

pub fn upper(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| single(c, c.to_uppercase()))
}

pub fn lower(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| single(c, c.to_lowercase()))
}

pub fn proper(_: &mut Evaluator, a: &[Value]) -> R {
    let mut prev_alnum = false;
    map_chars(a, |c| {
        let out = if prev_alnum {
            single(c, c.to_lowercase())
        } else {
            single(c, c.to_uppercase())
        };
        prev_alnum = c.is_alphanumeric();
        out
    })
}

pub fn replace(_: &mut Evaluator, a: &[Value]) -> R {
    let s = chars(a, 0);
    let start = clamp_len(int(a, 1)?.max(1) - 1, s.len());
    let n = clamp_len(int(a, 2)?, s.len() - start);
    let mut out: String = s[..start].iter().collect();
    out.push_str(&txt(a, 3));
    out.extend(&s[start + n..]);
    Ok(Value::Text(out))
}

pub fn filter(_: &mut Evaluator, a: &[Value]) -> R {
    let keep = txt(a, 1);
    Ok(Value::Text(
        txt(a, 0).chars().filter(|c| keep.contains(*c)).collect(),
    ))
}

pub fn exact(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::bool(txt(a, 0) == txt(a, 1)))
}

pub fn quote(_: &mut Evaluator, a: &[Value]) -> R {
    let mut out = String::from("\"");
    for c in txt(a, 0).chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '¶' => out.push_str("\\¶"),
            '\r' => out.push('¶'),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(Value::Text(out))
}

/// Increments the last run of digits, keeping its zero padding.
pub fn serial_increment(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let by = int(a, 1)?;
    let Some(end) = s.rfind(|c: char| c.is_ascii_digit()).map(|i| i + 1) else {
        return Ok(Value::Text(format!("{s}{by}")));
    };
    let begin = s[..end]
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    let digits = &s[begin..end];
    let n = digits
        .parse::<i128>()
        .unwrap_or(0)
        .saturating_add(by as i128);
    let width = digits.len();
    Ok(Value::Text(format!(
        "{}{n:0width$}{}",
        &s[..begin],
        &s[end..]
    )))
}

// ---- value lists ----------------------------------------------------------------

/// Values are separated by carriage returns; one trailing CR is ignored.
pub fn split_values(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return vec![];
    }
    let s = s.strip_suffix('\r').unwrap_or(s);
    s.split('\r').collect()
}

/// Each value followed by a CR, as LeftValues & co. return them.
fn join_values<'a>(v: impl IntoIterator<Item = &'a str>) -> R {
    Ok(Value::Text(
        v.into_iter().map(|s| format!("{s}\r")).collect(),
    ))
}

pub fn value_count(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(split_values(&txt(a, 0)).len() as i64))
}

pub fn get_value(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let n = int(a, 1)?;
    let v = split_values(&s);
    Ok(Value::text(if n >= 1 {
        v.get(n as usize - 1).copied().unwrap_or("")
    } else {
        ""
    }))
}

pub fn left_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let v = split_values(&s);
    join_values(v[..clamp_len(int(a, 1)?, v.len())].iter().copied())
}

pub fn right_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let v = split_values(&s);
    join_values(
        v[v.len() - clamp_len(int(a, 1)?, v.len())..]
            .iter()
            .copied(),
    )
}

pub fn middle_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let v = split_values(&s);
    let start = clamp_len(int(a, 1)?.max(1) - 1, v.len());
    let n = clamp_len(int(a, 2)?, v.len() - start);
    join_values(v[start..start + n].iter().copied())
}

pub fn filter_values(_: &mut Evaluator, a: &[Value]) -> R {
    let (s, f) = (txt(a, 0), txt(a, 1));
    let keep: Vec<String> = split_values(&f).iter().map(|v| v.to_lowercase()).collect();
    join_values(
        split_values(&s)
            .into_iter()
            .filter(|v| keep.contains(&v.to_lowercase())),
    )
}

/// Sort/compare key for SortValues and UniqueValues.
fn typed(s: &str, datatype: i64) -> Value {
    use crate::value::{parse_date, parse_time, parse_timestamp};
    let v = match datatype {
        2 => crate::value::text_to_num(s).map(Value::Number),
        3 => parse_date(s).map(Value::Date),
        4 => parse_time(s).map(Value::Time),
        5 => parse_timestamp(s).map(Value::Timestamp),
        _ => None,
    };
    v.unwrap_or_else(|| Value::text(s))
}

fn datatype(a: &[Value]) -> Result<i64, EvalError> {
    Ok(if a.len() > 1 && !a[1].is_empty() {
        int(a, 1)?
    } else {
        1
    })
}

// ponytail: the locale parameter is ignored; text sorts case-insensitively
// by code point, not by a locale collation.
pub fn sort_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let dt = datatype(a)?;
    let mut v = split_values(&s);
    if dt == 6 {
        v.sort();
    } else {
        v.sort_by(|x, y| compare(&typed(x, dt), &typed(y, dt)));
    }
    join_values(v)
}

pub fn unique_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let dt = datatype(a)?;
    let mut seen: Vec<Value> = vec![];
    let mut out = vec![];
    for v in split_values(&s) {
        let k = typed(v, dt);
        let dup = seen.iter().any(|x| {
            if dt == 6 {
                x.to_text() == v
            } else {
                compare(x, &k).is_eq()
            }
        });
        if !dup {
            seen.push(k);
            out.push(v);
        }
    }
    join_values(out)
}

// ---- words ------------------------------------------------------------------

/// (start, end) char ranges of words. Letters and digits form words; `.`,
/// `'` and `’` join two word characters, `,` joins two digits.
fn words(s: &[char]) -> Vec<(usize, usize)> {
    let w = |i: usize| s.get(i).is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let joins = |i: usize| {
        let (p, n) = (i.checked_sub(1).and_then(|p| s.get(p)), s.get(i + 1));
        match s[i] {
            '.' | '\'' | '’' => i > 0 && w(i - 1) && w(i + 1),
            ',' => p.is_some_and(|c| c.is_ascii_digit()) && n.is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        }
    };
    let mut out = vec![];
    let mut i = 0;
    while i < s.len() {
        if !w(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < s.len() && (w(i) || joins(i)) {
            i += 1;
        }
        out.push((start, i));
    }
    out
}

fn word_span(a: &[Value], first: i64, n: i64) -> R {
    let s = chars(a, 0);
    let w = words(&s);
    let first = clamp_len(first.max(1) - 1, w.len());
    let n = clamp_len(n, w.len() - first);
    if n == 0 {
        return Ok(Value::empty());
    }
    t(&s[w[first].0..w[first + n - 1].1])
}

pub fn left_words(_: &mut Evaluator, a: &[Value]) -> R {
    word_span(a, 1, int(a, 1)?)
}

pub fn middle_words(_: &mut Evaluator, a: &[Value]) -> R {
    word_span(a, int(a, 1)?, int(a, 2)?)
}

pub fn right_words(_: &mut Evaluator, a: &[Value]) -> R {
    let count = words(&chars(a, 0)).len() as i64;
    let n = int(a, 1)?.clamp(0, count);
    word_span(a, count - n + 1, n)
}

pub fn word_count(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(words(&chars(a, 0)).len() as i64))
}

// ---- Japanese width / kana -------------------------------------------------------

fn shift(c: char, from: std::ops::RangeInclusive<u32>, delta: i64) -> char {
    if from.contains(&(c as u32)) {
        char::from_u32((c as i64 + delta) as u32).unwrap_or(c)
    } else {
        c
    }
}

pub fn hiragana(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| shift(c, 0x30A1..=0x30F6, -0x60))
}

pub fn katakana(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| shift(c, 0x3041..=0x3096, 0x60))
}

pub fn roman_zenkaku(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| {
        if c == ' ' {
            '\u{3000}'
        } else {
            shift(c, 0x21..=0x7E, 0xFEE0)
        }
    })
}

pub fn roman_hankaku(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| {
        if c == '\u{3000}' {
            ' '
        } else {
            shift(c, 0xFF01..=0xFF5E, -0xFEE0)
        }
    })
}

// ---- styled text ------------------------------------------------------------------

// TODO(styled text): values carry no style runs yet, so the style functions
// return their text unchanged.
pub fn plain(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(txt(a, 0)))
}

/// TextStyleAdd/Remove: the style argument (Bold + Italic...) is not evaluated.
pub fn plain_lazy(ev: &mut Evaluator, a: &[Expr]) -> R {
    Ok(Value::Text(ev.eval(&a[0])?.to_text()))
}
