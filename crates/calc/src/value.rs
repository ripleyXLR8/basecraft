//! Calculation values, coercions and text formatting.
//!
//! Dates are day numbers (1/1/0001 = 1); times are seconds (fractional and
//! negative allowed); timestamps are seconds since 1/1/0001 00:00:00.
//! Text formats follow an en_US file: `M/D/YYYY`, times `H:MM:SS` and
//! timestamps `M/D/YYYY h:MM[:SS] AM` (checked against FileMaker).

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
    /// A date, time or timestamp shown in the form it was read in:
    /// FileMaker keeps a date's separator (`GetAsDate ( "1.15.2024" )`) and
    /// the 12-hour form of a time read with AM/PM or from a timestamp. It
    /// behaves as `value`; only its text differs. Calculations make it; the
    /// engine stores fields as [`Value::plain`].
    Shown {
        value: Box<Value>,
        text: String,
    },
}

/// A container value: a file name and its bytes, or a file stored by
/// reference (`path`, e.g. `imagewin:/C:/photo.jpg`, with no bytes).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Container {
    pub name: String,
    pub data: Vec<u8>,
    /// Path of a file stored by reference; empty for embedded data.
    pub path: String,
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
            Value::Container(c) => c.data.is_empty() && c.name.is_empty() && c.path.is_empty(),
            Value::Shown { value, .. } => value.is_empty(),
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
            // GetAsText: the path of a reference, else the file name.
            Value::Container(c) if !c.path.is_empty() => c.path.clone(),
            Value::Container(c) => c.name.clone(),
            Value::Shown { text, .. } => text.clone(),
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
            Value::Shown { value, .. } => value.to_num(),
        }
    }

    /// Numeric interpretation with empty treated as 0, as arithmetic does.
    pub fn num_or_zero(&self) -> Num {
        self.to_num().unwrap_or_else(|| num::int(0))
    }

    pub fn truth(&self) -> bool {
        match self {
            Value::Container(c) => !c.data.is_empty() || !c.name.is_empty() || !c.path.is_empty(),
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
            Value::Text(s) | Value::Json { text: s, .. } => parse_date(s).or_else(|| {
                let ts = s.trim().contains(' ').then(|| parse_timestamp(s));
                ts.flatten().map(|t| ts_date(&t))
            }),
            Value::Time(_) | Value::Container(_) => None,
            Value::Shown { value, .. } => value.to_date(),
        }
    }

    /// Time-of-day / duration interpretation used by Hour, Minute, Seconds.
    pub fn to_time(&self) -> Option<Num> {
        match self {
            Value::Time(t) => Some(t.clone()),
            Value::Timestamp(t) => Some(ts_time(t)),
            Value::Number(n) => Some(n.clone()),
            Value::Text(s) | Value::Json { text: s, .. } => parse_time(s).or_else(|| {
                let ts = s.trim().contains(' ').then(|| parse_timestamp(s));
                ts.flatten().map(|t| ts_time(&t))
            }),
            Value::Date(d) => Some(make_ts(*d, &num::int(0))),
            Value::Container(_) => None,
            Value::Shown { value, .. } => value.to_time(),
        }
    }

    /// The value without its shown form ([`Value::Shown`]).
    pub fn plain(self) -> Value {
        match self {
            Value::Shown { value, .. } => *value,
            v => v,
        }
    }

    /// [`Value::plain`] by reference.
    pub fn plain_ref(&self) -> &Value {
        match self {
            Value::Shown { value, .. } => value,
            v => v,
        }
    }
}

/// FileMaker's text-to-number rule: keep the digits (full-width ones too)
/// and the first decimal point, drop everything else; a minus sign or an
/// opening parenthesis before the first digit makes the number negative,
/// and `e` after a digit starts an exponent. GetAsNumber("abc12d3.4") =
/// 123.4, GetAsNumber("(5)") = -5, GetAsNumber("1e3") = 1000.
pub fn text_to_num(s: &str) -> Option<Num> {
    sci_text(s).or_else(|| gather_digits(s))
}

/// `1e3`, `-1.5E-2`, `.e3`: a number with one exponent and nothing else.
/// Otherwise an `e` is just another non-digit ("1e3e4" -> 134).
fn sci_text(s: &str) -> Option<Num> {
    let s = s.trim();
    let (m, e) = s.split_once(['e', 'E'])?;
    let e = e.strip_prefix('+').unwrap_or(e);
    let e_digits = e.strip_prefix('-').unwrap_or(e);
    let m_body = m.strip_prefix(['-', '+']).unwrap_or(m);
    let m_ok = !m_body.is_empty()
        && m_body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && m_body.matches('.').count() <= 1;
    if !m_ok || e_digits.is_empty() || !e_digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut mb = format!("0{m_body}");
    if mb.ends_with('.') {
        mb.push('0');
    }
    num::parse(&format!(
        "{}{mb}e{e}",
        if m.starts_with('-') { "-" } else { "" }
    ))
}

fn gather_digits(s: &str) -> Option<Num> {
    let s: Vec<char> = s
        .chars()
        .map(|c| match c {
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            c => c,
        })
        .collect();
    let mut out = String::new();
    let mut seen_digit = false;
    let mut seen_point = false;
    let mut neg = false;
    for c in s {
        if c.is_ascii_digit() {
            seen_digit = true;
            out.push(c);
        } else if c == '.' && !seen_point {
            seen_point = true;
            out.push('.');
        } else if (c == '-' || c == '(') && !seen_digit {
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

/// Fraction of a second as FileMaker shows it: at most 6 digits, truncated.
fn frac_text(frac: &Num) -> String {
    let f = num::trunc(frac, 6);
    if f.is_zero() {
        String::new()
    } else {
        // "0.25" -> ".25"
        num::plain(&f)[1..].to_string()
    }
}

/// `H:MM:SS[.ffffff]`, hours unbounded, as FileMaker shows a time.
pub fn format_time(t: &Num) -> String {
    let t = num::shown(t);
    let neg = t.is_negative();
    let a = t.abs();
    let whole = num::floor(&a);
    let s = whole.to_i128().unwrap_or(0);
    format!(
        "{}{}:{:02}:{:02}{}",
        if neg { "-" } else { "" },
        s / 3600,
        (s / 60) % 60,
        s % 60,
        frac_text(&(&a - &whole))
    )
}

/// Time of day in the 12-hour form of an en_US file, as timestamps show it:
/// `h:MM AM`, with `:SS[.ffffff]` only when there are seconds.
pub fn format_time12(t: &Num) -> String {
    let t = num::shown(t);
    let whole = num::floor(&t);
    let s = whole.to_i64().unwrap_or(0);
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    let frac = frac_text(&(&t - &whole));
    let (h12, suffix) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    if sec == 0 && frac.is_empty() {
        format!("{h12}:{m:02} {suffix}")
    } else {
        format!("{h12}:{m:02}:{sec:02}{frac} {suffix}")
    }
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
    format!("{} {}", format_date(ts_date(t)), format_time12(&ts_time(t)))
}

/// Parses `M/D/YYYY` with `/`, `-` or `.` as separator. Years of one or two
/// digits fall in the 100 years ending 30 years after the current one
/// (24 -> 2024, 68 -> 1968 in 2026); three or four digits are literal.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || "/-.".contains(c)) {
        return None;
    }
    let g: Vec<&str> = s.split(['/', '-', '.']).collect();
    if g.len() != 3 || g.iter().any(|p| p.is_empty() || p.len() > 4) {
        return None;
    }
    let (m, d, y): (i64, i64, i64) = (g[0].parse().ok()?, g[1].parse().ok()?, g[2].parse().ok()?);
    let y = if g[2].len() <= 2 {
        // In 2026 FileMaker reads 56 as 2056 and 57 as 1957.
        let pivot = current_year() + 30;
        let y = pivot / 100 * 100 + y;
        if y > pivot { y - 100 } else { y }
    } else {
        y
    };
    if !(1..=12).contains(&m) || d < 1 || !(MIN_YEAR..=MAX_YEAR).contains(&y) {
        return None;
    }
    let day = day_number(y, m, d);
    (civil(day).1 == m).then_some(day)
}

fn current_year() -> i64 {
    // The wasm32-unknown-unknown target has no clock.
    if cfg!(target_arch = "wasm32") {
        return 2026;
    }
    crate::functions::builtin_get("CurrentDate")
        .and_then(|v| v.to_date())
        .map_or(2026, |d| civil(d).0)
}

/// Parses a time: `H:MM`, `H:MM:SS[.fff]` (minutes and seconds may exceed
/// 59), optionally followed by AM/PM; `H AM`; a bare integer is hours and a
/// bare decimal seconds (FileMaker: "13" -> 13:00:00, "1.25" -> 1.25 s).
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
    let digits = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    let parts: Vec<&str> = body.split(':').collect();
    let t = if parts.len() == 1 {
        let p = parts[0];
        if digits(p) {
            num::int(p.parse::<i64>().ok()? * 3600)
        } else if pm.is_none() && p.chars().all(|c| c.is_ascii_digit() || c == '.') {
            num::parse(&format!("0{p}"))?
        } else {
            return None;
        }
    } else {
        if parts.len() > 3 || !digits(parts[0]) || !digits(parts[1]) {
            return None;
        }
        let sec = match parts.get(2) {
            Some(p) if p.chars().all(|c| c.is_ascii_digit() || c == '.') && !p.is_empty() => {
                num::parse(&format!("0{p}"))?
            }
            Some(_) => return None,
            None => num::int(0),
        };
        let (h, m): (i64, i64) = (parts[0].parse().ok()?, parts[1].parse().ok()?);
        num::fix(num::int(h * 3600 + m * 60) + sec)
    };
    let t = match pm {
        None => t,
        Some(pm) => {
            let h = num::floor_div(&t, &num::int(3600))?.to_i64()?;
            if !(0..=12).contains(&h) || neg {
                return None;
            }
            let base = if h == 12 { -43200 } else { 0 } + if pm { 43200 } else { 0 };
            num::fix(t + num::int(base))
        }
    };
    Some(if neg { -t } else { t })
}

/// `date time`, a date alone (midnight) or a time alone (on 1/1/0001); the
/// time of day must be under 24 hours.
pub fn parse_timestamp(s: &str) -> Option<Num> {
    let s = s.trim();
    let (d, t) = match s.split_once(' ') {
        Some((d, t)) => (parse_date(d)?, parse_time(t)?),
        None => match parse_date(s) {
            Some(d) => (d, num::int(0)),
            None if s.contains(':') => (1, parse_time(s)?),
            None => return None,
        },
    };
    (!t.is_negative() && t < num::int(DAY)).then(|| make_ts(d, &t))
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
        assert_eq!(parse_date("2024-01-15"), None);
        assert_eq!(parse_date("1-15-2024"), Some(day_number(2024, 1, 15)));
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
        assert_eq!(text_to_num("1e3"), num::parse("1000"));
        assert_eq!(text_to_num("1e3e4"), num::parse("134"));
        assert_eq!(text_to_num("(5)"), num::parse("-5"));
        assert_eq!(text_to_num("１２"), num::parse("12"));
    }
}
