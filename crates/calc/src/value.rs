//! Calculation values, coercions and text formatting.
//!
//! Dates are day numbers (1/1/0001 = 1); times are seconds (fractional and
//! negative allowed); timestamps are seconds since 1/1/0001 00:00:00.
//! Text formats follow an en_US file: `M/D/YYYY` and `H:MM:SS`.

use crate::num::{self, Num};
use bigdecimal::{Signed, ToPrimitive, Zero};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Number(Num),
    Date(i64),
    Time(Num),
    Timestamp(Num),
    Container(Container),
    /// Text that JSONParse or JSONSetElement produced. It behaves as text;
    /// `state` is what JSONParsedState reports: -1 invalid, or the JSON type
    /// (1 string ... 6 null).
    Json {
        text: String,
        state: i64,
    },
}

/// Minimal container: a file name and its bytes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Container {
    pub name: String,
    pub data: Vec<u8>,
}

pub const MIN_YEAR: i64 = 1;
pub const MAX_YEAR: i64 = 4000;

impl Value {
    pub fn empty() -> Value {
        Value::Text(String::new())
    }

    pub fn text(s: impl Into<String>) -> Value {
        Value::Text(s.into())
    }

    pub fn int(i: i64) -> Value {
        Value::Number(num::int(i))
    }

    pub fn num(n: Num) -> Value {
        Value::Number(num::fix(n))
    }

    pub fn bool(b: bool) -> Value {
        Value::int(b as i64)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Value::Text(s) | Value::Json { text: s, .. } => s.is_empty(),
            Value::Container(c) => c.data.is_empty() && c.name.is_empty(),
            _ => false,
        }
    }

    /// The value as FileMaker displays it in a calculation result.
    pub fn to_text(&self) -> String {
        match self {
            Value::Text(s) | Value::Json { text: s, .. } => s.clone(),
            Value::Number(n) => num::format(n),
            Value::Date(d) => format_date(*d),
            Value::Time(t) => format_time(t),
            Value::Timestamp(t) => format_timestamp(t),
            Value::Container(c) => c.name.clone(),
        }
    }

    /// Numeric interpretation; `None` when there is no number at all
    /// (empty text, text without digits, containers).
    pub fn to_num(&self) -> Option<Num> {
        match self {
            Value::Number(n) => Some(n.clone()),
            Value::Date(d) => Some(num::int(*d)),
            Value::Time(t) | Value::Timestamp(t) => Some(t.clone()),
            Value::Text(s) | Value::Json { text: s, .. } => text_to_num(s),
            Value::Container(_) => None,
        }
    }

    /// Numeric interpretation with empty treated as 0, as arithmetic does.
    pub fn num_or_zero(&self) -> Num {
        self.to_num().unwrap_or_else(|| num::int(0))
    }

    pub fn truth(&self) -> bool {
        match self {
            Value::Container(c) => !c.data.is_empty() || !c.name.is_empty(),
            _ => self.to_num().is_some_and(|n| !n.is_zero()),
        }
    }

    /// Date interpretation used by date functions (Year, Month...).
    pub fn to_date(&self) -> Option<i64> {
        match self {
            Value::Date(d) => Some(*d),
            Value::Timestamp(t) => Some(ts_date(t)),
            Value::Number(n) => n
                .with_scale_round(0, bigdecimal::RoundingMode::Floor)
                .to_i64(),
            Value::Text(s) | Value::Json { text: s, .. } => {
                parse_date(s).or_else(|| parse_timestamp(s).map(|t| ts_date(&t)))
            }
            Value::Time(_) | Value::Container(_) => None,
        }
    }

    /// Time-of-day / duration interpretation used by Hour, Minute, Seconds.
    pub fn to_time(&self) -> Option<Num> {
        match self {
            Value::Time(t) => Some(t.clone()),
            Value::Timestamp(t) => Some(ts_time(t)),
            Value::Number(n) => Some(n.clone()),
            Value::Text(s) | Value::Json { text: s, .. } => {
                parse_time(s).or_else(|| parse_timestamp(s).map(|t| ts_time(&t)))
            }
            Value::Date(_) | Value::Container(_) => None,
        }
    }
}

/// FileMaker's text-to-number rule: keep the digits and the first decimal
/// point, drop everything else; a minus sign before the first digit makes the
/// number negative. GetAsNumber("abc12d3.4") = 123.4.
// ponytail: no exponent ("1e3") support in text; add if real files need it.
pub fn text_to_num(s: &str) -> Option<Num> {
    let mut out = String::new();
    let mut seen_digit = false;
    let mut seen_point = false;
    let mut neg = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            seen_digit = true;
            out.push(c);
        } else if c == '.' && !seen_point {
            seen_point = true;
            out.push('.');
        } else if c == '-' && !seen_digit {
            neg = true;
        }
    }
    if !seen_digit {
        return None;
    }
    let n = num::parse(&format!("0{out}"))?;
    Some(if neg { -n } else { n })
}

// ---- calendar ---------------------------------------------------------

/// Day number (1/1/0001 = 1) of a proleptic Gregorian date. Months outside
/// 1..12 and days outside the month roll over, like FileMaker's Date().
pub fn day_number(year: i64, month: i64, day: i64) -> i64 {
    let y = year + (month - 1).div_euclid(12);
    let m = (month - 1).rem_euclid(12) + 1;
    days_from_civil(y, m, 1) + day - 1
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468 + 719163
}

/// (year, month, day) of a day number.
pub fn civil(day: i64) -> (i64, i64, i64) {
    let z = day - 719163 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

pub fn valid_day(day: i64) -> bool {
    (1..=days_from_civil(MAX_YEAR, 12, 31)).contains(&day)
}

/// Sunday = 1 ... Saturday = 7. 1/1/0001 was a Monday.
pub fn day_of_week(day: i64) -> i64 {
    day.rem_euclid(7) + 1
}

pub fn format_date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{m}/{d}/{y:04}")
}

pub fn format_time(t: &Num) -> String {
    let neg = t.is_negative();
    let a = t.abs();
    let whole = num::floor(&a);
    let frac = num::fix(&a - &whole);
    let s = whole.to_i128().unwrap_or(0);
    let mut out = format!(
        "{}{}:{:02}:{:02}",
        if neg { "-" } else { "" },
        s / 3600,
        (s / 60) % 60,
        s % 60
    );
    if !frac.is_zero() {
        // "0.25" -> ".25"
        out.push_str(&num::format(&frac)[1..]);
    }
    out
}

const DAY: i64 = 86400;

pub fn ts_date(t: &Num) -> i64 {
    num::floor_div(t, &num::int(DAY))
        .and_then(|d| d.to_i64())
        .unwrap_or(0)
        .saturating_add(1)
}

pub fn ts_time(t: &Num) -> Num {
    num::modulo(t, &num::int(DAY)).unwrap_or_default()
}

pub fn make_ts(day: i64, time: &Num) -> Num {
    num::fix(num::int((day - 1) * DAY) + time)
}

pub fn format_timestamp(t: &Num) -> String {
    format!("{} {}", format_date(ts_date(t)), format_time(&ts_time(t)))
}

/// Parses `M/D/YYYY` (any of `/ - .` as separator) or `YYYY-MM-DD`.
// ponytail: en_US order only; the file's locale should drive this later.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || "/-. ".contains(c)) {
        return None;
    }
    let g: Vec<&str> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .collect();
    if g.len() != 3 || g.iter().any(|p| p.len() > 4) {
        return None;
    }
    let (ys, ms, ds) = if g[0].len() == 4 {
        (g[0], g[1], g[2])
    } else {
        (g[2], g[0], g[1])
    };
    let (y, m, d): (i64, i64, i64) = (ys.parse().ok()?, ms.parse().ok()?, ds.parse().ok()?);
    // ponytail: two-digit years map to 1930..2029; FileMaker uses a rolling window.
    let y = match (ys.len() <= 2, y < 30) {
        (true, true) => 2000 + y,
        (true, false) => 1900 + y,
        _ => y,
    };
    if !(1..=12).contains(&m) || d < 1 || !(MIN_YEAR..=MAX_YEAR).contains(&y) {
        return None;
    }
    let day = day_number(y, m, d);
    (civil(day).1 == m).then_some(day)
}

/// Parses `H:MM`, `H:MM:SS`, `H:MM:SS.fff`, optionally followed by AM/PM.
pub fn parse_time(s: &str) -> Option<Num> {
    let mut s = s.trim().to_ascii_uppercase();
    let mut pm = None;
    for (suffix, is_pm) in [("PM", true), ("AM", false)] {
        if let Some(rest) = s.strip_suffix(suffix) {
            pm = Some(is_pm);
            s = rest.trim_end().to_string();
            break;
        }
    }
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s.as_str()),
    };
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let ok = |p: &str, frac: bool| {
        !p.is_empty() && p.chars().all(|c| c.is_ascii_digit() || (frac && c == '.'))
    };
    if !ok(parts[0], false) || !ok(parts[1], false) || (parts.len() == 3 && !ok(parts[2], true)) {
        return None;
    }
    let mut h: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let sec = if parts.len() == 3 {
        num::parse(parts[2])?
    } else {
        num::int(0)
    };
    if let Some(pm) = pm {
        if !(1..=12).contains(&h) {
            return None;
        }
        h = h % 12 + if pm { 12 } else { 0 };
    }
    let t = num::fix(num::int(h * 3600 + m * 60) + sec);
    Some(if neg { -t } else { t })
}

pub fn parse_timestamp(s: &str) -> Option<Num> {
    let (d, t) = s.trim().split_once(' ')?;
    Some(make_ts(parse_date(d)?, &parse_time(t)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trip() {
        assert_eq!(day_number(1, 1, 1), 1);
        assert_eq!(civil(1), (1, 1, 1));
        assert_eq!(day_of_week(1), 2); // Monday
        for d in [1, 59, 60, 61, 365, 366, 730_000, 738_000, 1_460_000] {
            let (y, m, dd) = civil(d);
            assert_eq!(day_number(y, m, dd), d);
        }
        assert_eq!(format_date(day_number(2024, 1, 15)), "1/15/2024");
        assert_eq!(day_number(2023, 13, 1), day_number(2024, 1, 1));
        assert_eq!(day_number(2024, 3, 0), day_number(2024, 2, 29));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_date("1/15/2024"), Some(day_number(2024, 1, 15)));
        assert_eq!(parse_date("2024-01-15"), Some(day_number(2024, 1, 15)));
        assert_eq!(parse_date("2/30/2024"), None);
        assert_eq!(parse_date("abc"), None);
        assert_eq!(parse_time("1:30 PM"), num::parse("48600"));
        assert_eq!(parse_time("12:00 AM"), num::parse("0"));
        assert_eq!(parse_time("10:00:01.5"), num::parse("36001.5"));
        assert_eq!(format_time(&num::parse("36001.5").unwrap()), "10:00:01.5");
        assert_eq!(format_time(&num::parse("-3600").unwrap()), "-1:00:00");
        assert_eq!(format_time(&num::parse("90000").unwrap()), "25:00:00");
        assert_eq!(text_to_num("abc12d3.4"), num::parse("123.4"));
        assert_eq!(text_to_num("-5"), num::parse("-5"));
        assert_eq!(text_to_num("abc"), None);
        assert_eq!(text_to_num(".5"), num::parse("0.5"));
    }
}
