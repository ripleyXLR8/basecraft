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

/// Characters of text; bytes of a container.
pub fn length(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::int(match &a[0] {
        Value::Container(c) => c.data.len(),
        v => v.to_text().chars().count(),
    } as i64))
}

fn folded(s: &[char]) -> Vec<char> {
    s.iter().map(|&c| fold(c)).collect()
}

fn match_at(hay: &[char], needle: &[char], i: usize) -> bool {
    hay.get(i..i + needle.len()) == Some(needle)
}

/// Position: case-insensitive, occurrences never overlap (Position (
/// "aaaa" ; "aa" ; 1 ; 2 ) = 3). A negative occurrence searches backward
/// from `start`.
pub fn position(_: &mut Evaluator, a: &[Value]) -> R {
    let (s, needle) = (folded(&chars(a, 0)), folded(&chars(a, 1)));
    let (start, occ) = (int(a, 2)?, int(a, 3)?);
    if needle.is_empty() || s.is_empty() || occ == 0 {
        return Ok(Value::int(0));
    }
    let mut left = occ.unsigned_abs();
    if occ > 0 {
        let mut i = (start.max(1) - 1) as usize;
        while i + needle.len() <= s.len() {
            if match_at(&s, &needle, i) {
                left -= 1;
                if left == 0 {
                    return Ok(Value::int(i as i64 + 1));
                }
                i += needle.len();
            } else {
                i += 1;
            }
        }
    } else {
        let mut i = start.min(s.len() as i64) - 1;
        while i >= 0 {
            if match_at(&s, &needle, i as usize) {
                left -= 1;
                if left == 0 {
                    return Ok(Value::int(i + 1));
                }
                i -= needle.len() as i64;
            } else {
                i -= 1;
            }
        }
    }
    Ok(Value::int(0))
}

/// Non-overlapping, case-insensitive: PatternCount ( "aaaa" ; "aa" ) = 2.
pub fn pattern_count(_: &mut Evaluator, a: &[Value]) -> R {
    let (s, needle) = (folded(&chars(a, 0)), folded(&chars(a, 1)));
    let (mut n, mut i) = (0, 0);
    while !needle.is_empty() && i + needle.len() <= s.len() {
        if match_at(&s, &needle, i) {
            n += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    Ok(Value::int(n))
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

/// Characters Trim removes: spaces of every width (no-break, en/em,
/// ideographic, zero-width), not tabs or line breaks.
fn trim_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{a0}' | '\u{2000}'..='\u{200b}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

pub fn trim(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::text(txt(a, 0).trim_matches(trim_space)))
}

/// Japanese (non-Roman) characters for TrimAll: CJK, kana (half-width
/// too), CJK punctuation and full-width forms.
fn japanese(c: char) -> bool {
    c >= '\u{2E80}'
}

/// TrimAll: trims both ends and collapses runs of spaces to one (with
/// `trimSpaces` 1, full-width spaces count and become half-width). Between
/// two Roman characters the space stays; `trimType` 3 removes every space,
/// 0 and 1 the ones next to a Japanese character, and 1 puts one between
/// Japanese and Roman text; 2 changes nothing more.
pub fn trim_all(_: &mut Evaluator, a: &[Value]) -> R {
    let full_width = int(a, 1)? != 0;
    let kind = int(a, 2)?;
    let is_space = |c: char| c == ' ' || (full_width && c == '\u{3000}');
    let mut out: Vec<char> = Vec::new();
    let mut pending = false;
    for c in txt(a, 0).chars() {
        if is_space(c) {
            pending = !out.is_empty();
            continue;
        }
        let space = match out.last() {
            None => false,
            Some(&p) => match (japanese(p), japanese(c), kind) {
                (_, _, 3) | (true, true, _) => false,
                (false, false, _) | (_, _, 2) => pending,
                _ => kind == 1,
            },
        };
        if space {
            out.push(' ');
        }
        pending = false;
        out.push(c);
    }
    t(&out)
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

/// İ lowers to "i" plus a combining dot; FileMaker keeps the "i".
pub fn lower(_: &mut Evaluator, a: &[Value]) -> R {
    map_chars(a, |c| c.to_lowercase().next().unwrap_or(c))
}

/// Proper: a letter is capitalized unless it follows a letter, a digit, a
/// period, an apostrophe or a no-break space ("o'neil" -> "O'neil",
/// "foo_bar" -> "Foo_Bar").
pub fn proper(_: &mut Evaluator, a: &[Value]) -> R {
    let mut inside = false;
    map_chars(a, |c| {
        let out = if inside {
            c.to_lowercase().next().unwrap_or(c)
        } else {
            single(c, c.to_uppercase())
        };
        inside = c.is_alphanumeric() || matches!(c, '.' | '\'' | '’' | '\u{a0}');
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

/// Quote: `"` becomes `\"`, a pilcrow `\¶` and a carriage return `¶`; a
/// backslash is doubled only before one of those or at the end.
pub fn quote(_: &mut Evaluator, a: &[Value]) -> R {
    let s = chars(a, 0);
    let mut out = String::from("\"");
    for (i, &c) in s.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' if matches!(s.get(i + 1), None | Some('¶' | '"' | '\r')) => out.push_str("\\\\"),
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
    // The zero padding goes in front of the sign: "abc005" - 10 = "abc0-5".
    let n = n.to_string();
    let pad = "0".repeat(digits.len().saturating_sub(n.len()));
    Ok(Value::Text(format!("{}{pad}{n}{}", &s[..begin], &s[end..])))
}

// ---- value lists ----------------------------------------------------------------

/// Line breaks that separate values: CR, LF (so CR LF makes an empty
/// value between them) and the Unicode line and paragraph separators.
fn value_sep(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}')
}

/// Values are separated by line breaks; one trailing break is ignored.
pub fn split_values(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return vec![];
    }
    let s = s.strip_suffix(value_sep).unwrap_or(s);
    s.split(value_sep).collect()
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

/// Case-insensitive, accent-sensitive ("é" is not "e").
pub fn filter_values(_: &mut Evaluator, a: &[Value]) -> R {
    let (s, f) = (txt(a, 0), txt(a, 1));
    let keep: Vec<String> = split_values(&f).iter().map(|v| v.to_lowercase()).collect();
    join_values(
        split_values(&s)
            .into_iter()
            .filter(|v| keep.contains(&v.to_lowercase())),
    )
}

/// Sort/compare key for SortValues and UniqueValues; `None` when the value
/// is not of the type.
fn typed_key(s: &str, datatype: i64) -> Option<Value> {
    use crate::value::{parse_date, parse_time, parse_timestamp};
    match datatype.abs() {
        2 => crate::value::text_to_num(s).map(Value::Number),
        3 => parse_date(s).map(Value::Date),
        4 => parse_time(s).map(Value::Time),
        5 => parse_timestamp(s).map(Value::Timestamp),
        _ => Some(Value::text(s)),
    }
}

fn typed(s: &str, datatype: i64) -> Value {
    typed_key(s, datatype).unwrap_or_else(|| Value::text(s))
}

/// Base letter of common accented Latin letters, for collation.
fn base_letter(c: char) -> char {
    const FROM: &str = "àáâãäåçèéêëìíîïñòóôõöøùúûüýÿ";
    const TO: &str = "aaaaaaceeeeiiiinoooooouuuuyy";
    let l = fold(c);
    FROM.chars()
        .position(|x| x == l)
        .and_then(|i| TO.chars().nth(i))
        .unwrap_or(l)
}

/// Case- and accent-insensitive key ("Straße" = "STRASSE").
pub(crate) fn base_key(s: &str) -> String {
    s.chars()
        .flat_map(|c| match base_letter(c) {
            'ß' => vec!['s', 's'],
            b => vec![b],
        })
        .collect()
}

/// Text order: case- and accent-insensitive first, then accents
/// (e < é < f).
pub(crate) fn collate(x: &str, y: &str) -> std::cmp::Ordering {
    base_key(x)
        .cmp(&base_key(y))
        .then_with(|| x.to_lowercase().cmp(&y.to_lowercase()))
}

fn compare_keys(x: &Value, y: &Value) -> std::cmp::Ordering {
    match (x, y) {
        (Value::Text(a), Value::Text(b)) => collate(a, b),
        _ => compare(x, y),
    }
}

/// 1 text, 2 number, 3 date, 4 time, 5 timestamp, negative for descending;
/// anything else is ascending text.
fn datatype(a: &[Value]) -> Result<i64, EvalError> {
    let dt = if a.len() > 1 && !a[1].is_empty() {
        int(a, 1)?
    } else {
        1
    };
    Ok(if (1..=5).contains(&dt.abs()) { dt } else { 1 })
}

/// SortValues: a negative type sorts descending. Values that are not of a
/// number/date/time type keep their order, after the others ascending and
/// before them descending.
// ponytail: the locale parameter is ignored; text uses a simple Latin
// collation (case- and accent-insensitive first).
pub fn sort_values(_: &mut Evaluator, a: &[Value]) -> R {
    use std::cmp::Ordering::Equal;
    let s = txt(a, 0);
    let dt = datatype(a)?;
    let mut v: Vec<(&str, Option<Value>)> = split_values(&s)
        .into_iter()
        .map(|x| (x, typed_key(x, dt)))
        .collect();
    let desc = dt < 0;
    // Values that are not of the type keep their order. Ascending, they
    // follow the others; descending, those without any digit come first
    // and those with digits ("2" as a date) last (FileMaker 26).
    let class = |(x, k): &(&str, Option<Value>)| match (k, desc) {
        (Some(_), false) => 0,
        (None, false) => 1,
        (None, true) if crate::value::text_to_num(x).is_none() => 0,
        (Some(_), true) => 1,
        (None, true) => 2,
    };
    v.sort_by(|a, b| {
        class(a).cmp(&class(b)).then_with(|| match (&a.1, &b.1) {
            (Some(x), Some(y)) if desc => compare_keys(y, x),
            (Some(x), Some(y)) => compare_keys(x, y),
            _ => Equal,
        })
    });
    join_values(v.into_iter().map(|(x, _)| x))
}

/// Text values are equal when they differ only in case or accents
/// ("é", "e" and "E" are one value).
pub fn unique_values(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let dt = datatype(a)?;
    let mut seen: Vec<Value> = vec![];
    let mut out = vec![];
    for v in split_values(&s) {
        let k = typed(v, dt);
        let dup = seen.iter().any(|x| match (x, &k) {
            (Value::Text(p), Value::Text(q)) => base_key(p) == base_key(q),
            _ => compare(x, &k).is_eq(),
        });
        if !dup {
            seen.push(k);
            out.push(v);
        }
    }
    join_values(out)
}

// ---- words ------------------------------------------------------------------

/// Script of a word character, for breaking Japanese text into words:
/// kanji, hiragana and katakana runs are separate words.
fn script(c: char) -> u8 {
    match c as u32 {
        0x3040..=0x309F => 1,
        0x30A0..=0x30FF | 0xFF66..=0xFF9F => 2,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => 3,
        _ => 0,
    }
}

/// (start, end) char ranges of words. Letters and digits form words (`_`
/// and `-` separate them); `'` and `’` join two letters, `.` two letters
/// or two digits, `,` `:` `/` two digits ("1:30", "1/2/2003", "10.5.3").
fn words(s: &[char]) -> Vec<(usize, usize)> {
    let w = |i: usize| s.get(i).is_some_and(|c| c.is_alphanumeric());
    let digit = |i: usize| s.get(i).is_some_and(|c| c.is_numeric());
    let letter = |i: usize| s.get(i).is_some_and(|c| c.is_alphabetic());
    let joins = |i: usize| {
        if i == 0 {
            return false;
        }
        match s[i] {
            '\'' | '’' => w(i - 1) && w(i + 1),
            '.' => (letter(i - 1) && letter(i + 1)) || (digit(i - 1) && digit(i + 1)),
            ',' | ':' | '/' => digit(i - 1) && digit(i + 1),
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
        i += 1;
        while i < s.len() && (w(i) || joins(i)) && script(s[i]) == script(s[i - 1]) {
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

/// HTML 4 names of U+00A0..U+00FF, which GetAsCSS uses (é -> &eacute;).
const LATIN1_ENTITIES: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf",
    "laquo", "not", "shy", "reg", "macr", "deg", "plusmn", "sup2", "sup3", "acute", "micro",
    "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest",
    "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute",
    "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH", "Ntilde", "Ograve", "Oacute",
    "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute",
    "THORN", "szlig", "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil",
    "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml", "eth", "ntilde",
    "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc",
    "uuml", "yacute", "thorn", "yuml",
];

/// HTML 4 names of common characters above U+00FF.
const SPECIAL_ENTITIES: [(char, &str); 25] = [
    ('Œ', "OElig"),
    ('œ', "oelig"),
    ('Š', "Scaron"),
    ('š', "scaron"),
    ('Ÿ', "Yuml"),
    ('ƒ', "fnof"),
    ('ˆ', "circ"),
    ('˜', "tilde"),
    ('–', "ndash"),
    ('—', "mdash"),
    ('‘', "lsquo"),
    ('’', "rsquo"),
    ('‚', "sbquo"),
    ('“', "ldquo"),
    ('”', "rdquo"),
    ('„', "bdquo"),
    ('†', "dagger"),
    ('‡', "Dagger"),
    ('•', "bull"),
    ('…', "hellip"),
    ('‰', "permil"),
    ('‹', "lsaquo"),
    ('›', "rsaquo"),
    ('€', "euro"),
    ('™', "trade"),
];

/// Escapes markup characters, CR as `<br/>`, other control characters and
/// non-ASCII characters as `&#N;`, or by HTML 4 name when there is one
/// (é -> &eacute;, € -> &euro;).
// ponytail: Greek and math entity names (&alpha;...) are not listed.
fn html_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\r' => out.push_str("<br/>"),
            c if (c as u32) < 0x20 => out.push_str(&format!("&#{};", c as u32)),
            '\u{a0}'..='\u{ff}' => {
                out.push('&');
                out.push_str(LATIN1_ENTITIES[c as usize - 0xA0]);
                out.push(';');
            }
            c if (c as u32) < 0x80 => out.push(c),
            c => match SPECIAL_ENTITIES.iter().find(|(x, _)| *x == c) {
                Some((_, name)) => out.push_str(&format!("&{name};")),
                None => out.push_str(&format!("&#{};", c as u32)),
            },
        }
    }
    out
}

/// GetAsCSS: styled runs become `<span style="...">`; text without styles
/// (all we keep) is one span with an empty style.
pub fn get_as_css(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    if s.is_empty() {
        return Ok(Value::empty());
    }
    Ok(Value::Text(format!(
        "<span style=\"\" >{}</span>",
        html_escape(&s)
    )))
}

/// GetAsSVG: a `<stylelist>` of runs and a `<data>` block of spans. Without
/// style runs the whole text is one run with an empty style.
pub fn get_as_svg(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    if s.is_empty() {
        return Ok(Value::empty());
    }
    let n = s.chars().count();
    Ok(Value::Text(format!(
        "<stylelist>\r<style#0>\"\", begin: 1, end: {n}</style> \r</stylelist>\r<data>\r<span style=\"0\">{}</span>\r</data>",
        html_escape(&s)
    )))
}
