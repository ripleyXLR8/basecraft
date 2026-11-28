//! A field object's data formats applied to a value, as FileMaker Pro 26
//! shows it in Browse mode, Preview mode and print (FORMAT.md §10, "Data
//! formatting"; each rule checked against FileMaker's rendering of the same
//! value with the same format bytes).

use calc::Value;
use fmp12::formats::{Format, num, time};
use fmp12::schema::{DateOrder, Locale};

/// The text a field shows for `v`, and the negative colour (RGB) when the
/// number format asks for it. `None`: the value shows as stored.
pub fn apply(f: &Format, v: &Value, loc: &Locale) -> Option<(String, Option<[u8; 3]>)> {
    match v {
        Value::Number(n) => {
            use bigdecimal::ToPrimitive;
            number(f, n.to_f64()?, loc)
        }
        Value::Date(d) => date(f, *d, loc).map(|s| (s, None)),
        Value::Time(t) => {
            use bigdecimal::ToPrimitive;
            clock(f, t.to_f64()?, loc).map(|s| (s, None))
        }
        Value::Timestamp(ts) => {
            use bigdecimal::ToPrimitive;
            // Formatted only with a custom time format (FileMaker shows the
            // stored text otherwise, whatever the date format).
            let t = clock(f, calc::value::ts_time(ts).to_f64()?, loc)?;
            let day = calc::value::ts_date(ts);
            let d =
                date(f, day, loc).unwrap_or_else(|| engine::stored_text(&Value::Date(day), loc));
            Some((format!("{d} {t}"), None))
        }
        _ => None,
    }
}

/// `n` with `digits` decimals, thousands grouped with `sep` (none if `None`).
fn digits(n: f64, fixed: Option<u16>, decimal: char, sep: Option<char>) -> String {
    let s = match fixed {
        Some(d) => format!("{:.*}", d as usize, n),
        None => {
            let s = format!("{n}");
            if s.contains('e') {
                format!("{n:.10}")
            } else {
                s
            }
        }
    };
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0
            && (int.len() - i) % 3 == 0
            && let Some(t) = sep
        {
            out.push(t);
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(decimal);
        out.push_str(frac);
    }
    out
}

fn number(f: &Format, n: f64, loc: &Locale) -> Option<(String, Option<[u8; 3]>)> {
    let o = &f.number;
    let flags = o.flags;
    if flags & (num::AS_ENTERED | num::GENERAL) != 0 {
        return None;
    }
    if flags & num::BOOLEAN != 0 {
        let t = if n != 0.0 {
            &o.true_text
        } else {
            &o.false_text
        };
        return Some((t.clone(), None));
    }
    if flags & num::HIDE_ZERO != 0 && n == 0.0 {
        return Some((String::new(), None));
    }
    let percent = flags & num::NOTATION != 0 && flags & num::PERCENT != 0;
    let currency = flags & num::NOTATION != 0 && !percent;
    let system = flags & num::SYSTEM_SEPARATORS != 0;
    let (dec, thousands) = if system {
        (loc.decimal, Some(loc.thousands))
    } else {
        let t = (flags & num::THOUSANDS != 0).then_some(o.thousands);
        (o.decimal, t)
    };
    let fixed = (flags & num::FIXED_DIGITS != 0).then_some(o.digits);
    let v = if percent { n.abs() * 100.0 } else { n.abs() };
    let mut s = digits(v, fixed, dec, thousands);
    if percent {
        s.push('%');
    }
    let neg = n < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0');
    let leading = flags & num::LEADING != 0;
    let outside = flags & num::SYMBOL_OUTSIDE != 0 && leading;
    if currency && !outside {
        s = if leading {
            format!("{}{s}", o.currency)
        } else {
            format!("{s}{}", o.currency)
        };
    }
    if neg {
        s = match o.negative_style {
            1 => format!("{s}-"),
            2 => format!("({s})"),
            3 => format!("<{s}>"),
            4 => format!("{s} CR"),
            5 => format!("\u{25B2}{s}"),
            // Styles 6 and 7 show no sign in FileMaker 26.
            6 | 7 => s,
            _ => format!("-{s}"),
        };
    }
    if currency && outside {
        s = format!("{}{s}", o.currency);
    }
    let color = (neg && flags & num::NEGATIVE_COLOR != 0).then_some(o.negative_color);
    Some((s, color))
}

fn french(loc: &Locale) -> bool {
    loc.language == 24
}

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_FR: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const DAYS_EN: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const DAYS_FR: [&str; 7] = [
    "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
];

fn month(m: i64, abbr: bool, loc: &Locale) -> String {
    let i = (m - 1).clamp(0, 11) as usize;
    match (french(loc), abbr) {
        // FileMaker 26 writes French month names in full in its abbreviated
        // formats too ("28 novembre 2026" for format 4).
        (true, _) => MONTHS_FR[i].into(),
        (false, true) => MONTHS_EN[i][..3].into(),
        (false, false) => MONTHS_EN[i].into(),
    }
}

fn weekday(day: i64, abbr: bool, loc: &Locale) -> String {
    let i = (calc::value::day_of_week(day) - 1).rem_euclid(7) as usize;
    match (french(loc), abbr) {
        (true, true) => format!("{}.", &DAYS_FR[i][..3]),
        (true, false) => DAYS_FR[i].into(),
        (false, true) => DAYS_EN[i][..3].into(),
        (false, false) => DAYS_EN[i].into(),
    }
}

/// Day or month number: style 1 leading zero, 2 leading space.
fn padded(n: i64, style: u8) -> String {
    match style {
        1 => format!("{n:02}"),
        2 => format!("{n:>2}"),
        _ => n.to_string(),
    }
}

fn date(f: &Format, day: i64, loc: &Locale) -> Option<String> {
    let o = &f.date;
    let (y, m, d) = calc::value::civil(day);
    let fr = french(loc);
    let long = |abbr_month: bool| {
        if fr {
            format!("{d} {} {y}", month(m, abbr_month, loc))
        } else {
            format!("{} {d}, {y}", month(m, abbr_month, loc))
        }
    };
    let numeric = |year: String| {
        let sep = if o.separator == '\0' {
            String::new()
        } else {
            o.separator.to_string()
        };
        let (dd, mm) = (padded(d, o.day_style), padded(m, o.month_style));
        match loc.date_order {
            DateOrder::Mdy => format!("{mm}{sep}{dd}{sep}{year}"),
            DateOrder::Dmy => format!("{dd}{sep}{mm}{sep}{year}"),
            DateOrder::Ymd => format!("{year}{sep}{mm}{sep}{dd}"),
        }
    };
    Some(match o.format {
        1 => {
            let mut s = o.separators[0].clone();
            for (k, e) in o.elements.iter().enumerate() {
                s.push_str(&match e {
                    1 => padded(d, o.day_style),
                    2 => weekday(day, true, loc),
                    3 => weekday(day, false, loc),
                    4 => padded(m, o.month_style),
                    5 => month(m, true, loc),
                    6 => month(m, false, loc),
                    7 => format!("{:02}", y % 100),
                    8 => y.to_string(),
                    9 if fr => format!("T{}", (m - 1) / 3 + 1),
                    9 => format!("Q{}", (m - 1) / 3 + 1),
                    10 if fr => match (m - 1) / 3 {
                        0 => "1er trimestre".into(),
                        q => format!("{}e trimestre", q + 1),
                    },
                    10 => {
                        let q = (m - 1) / 3 + 1;
                        let th = ["st", "nd", "rd", "th"][(q - 1) as usize];
                        format!("{q}{th} Quarter")
                    }
                    _ => String::new(),
                });
                s.push_str(&o.separators[k + 1]);
            }
            s
        }
        2 => numeric(format!("{:02}", y % 100)),
        7 => numeric(y.to_string()),
        3 => long(false),
        4 => long(true),
        5 if fr => format!("{} {}", weekday(day, true, loc), long(true)),
        5 => format!("{}, {}", weekday(day, true, loc), long(true)),
        6 | 9 | 10 | 11 if fr => format!("{} {}", weekday(day, false, loc), long(false)),
        6 | 9 | 10 | 11 => format!("{}, {}", weekday(day, false, loc), long(false)),
        // 0 as entered, 8 the system's short format: the stored text.
        _ => return None,
    })
}

/// A custom time format: hours, minutes, seconds as asked, 12-hour clock
/// with the AM/PM strings. Hours style and minutes/seconds style: 0 none,
/// 1 leading zero, 2 leading space. Without hours, the minutes count from
/// midnight. Seconds take the number format's fixed decimals.
fn clock(f: &Format, secs: f64, loc: &Locale) -> Option<String> {
    let o = &f.time;
    if o.flags & time::FORMATTED == 0
        || o.flags & time::H24 != 0
        || o.flags & (time::HOURS | time::MINUTES | time::SECONDS) == 0
    {
        return None;
    }
    let total = secs.floor() as i64;
    let (h, mi, s) = (total / 3600, total / 60 % 60, total % 60);
    let hours = o.flags & time::HOURS != 0;
    let pm = h % 24 >= 12;
    let h12 = match h % 12 {
        0 => 12,
        x => x,
    };
    let mut parts = Vec::new();
    if hours {
        parts.push(padded(h12, o.hour_style));
    }
    if o.flags & time::MINUTES != 0 {
        let m = if hours { mi } else { total / 60 };
        parts.push(padded(m, o.minsec_style));
    }
    if o.flags & time::SECONDS != 0 {
        let mut x = padded(s, o.minsec_style);
        let n = &f.number;
        if n.flags & num::FIXED_DIGITS != 0 && n.digits > 0 {
            let dec = if n.flags & num::SYSTEM_SEPARATORS != 0 {
                loc.decimal
            } else {
                n.decimal
            };
            let frac = secs - secs.floor();
            let d = format!("{:.*}", n.digits as usize, frac);
            x.push(dec);
            x.push_str(d.split_once('.').map_or("", |p| p.1));
        }
        parts.push(x);
    }
    let sep = o.separator.to_string();
    let suffix = if pm { &o.pm } else { &o.am };
    Some(format!("{}{suffix}", parts.join(&sep)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr() -> Locale {
        Locale {
            language: 24,
            thousands: '\u{a0}',
            decimal: ',',
            date_order: DateOrder::Dmy,
            am: String::new(),
            pm: String::new(),
            ..Locale::default()
        }
    }

    fn fmt(flags: u16, neg: u8) -> Format {
        let mut f = Format::new_default();
        f.number.flags = flags;
        f.number.negative_style = neg;
        f.number.currency = "€".into();
        f.number.digits = 2;
        f
    }

    fn show(f: &Format, v: Value) -> String {
        apply(f, &v, &fr())
            .map(|x| x.0)
            .unwrap_or_else(|| "=".into())
    }

    fn n(s: &str) -> Value {
        Value::Number(s.parse().unwrap())
    }

    /// FileMaker Pro 26's rendering of -1234.5678 (French file), one format
    /// option at a time (thousands ",", decimal ".", 2 digits, "€").
    #[test]
    fn numbers_as_filemaker_shows_them() {
        let cases: &[(u16, u8, &str)] = &[
            (0x0001, 0, "="),
            (0x0002, 0, "-1,234.5678"),
            (0x0004, 0, "-1234.5678€"),
            (0x0008, 0, "-1234.57"),
            (0x0020, 0, "Yes"),
            (0x0100, 0, "="),
            (0x0A0E, 0, "-€1,234.57"),
            (0x0A08, 0, "-1234.57"),
            (0x0A04, 0, "-€1234.5678"),
            (0x0A1C, 0, "-123456.78%"),
            (0x0E0C, 0, "€-1234.57"),
            (0x020C, 0, "-1234.57€"),
            (0x080C, 1, "€1234.57-"),
            (0x080C, 2, "(€1234.57)"),
            (0x080C, 3, "<€1234.57>"),
            (0x080C, 4, "€1234.57 CR"),
            (0x080C, 5, "\u{25B2}€1234.57"),
            (0x080C, 6, "€1234.57"),
        ];
        for (flags, neg, want) in cases {
            assert_eq!(
                show(&fmt(*flags, *neg), n("-1234.5678")),
                *want,
                "{flags:#x} {neg}"
            );
        }
        assert_eq!(show(&fmt(0x4A0C, 0), n("-1234.5678")), "-€1\u{a0}234,57");
        let red = apply(&fmt(0x1A0C, 0), &n("-1234.5678"), &fr()).unwrap();
        assert_eq!(red.1, Some([0xDD, 0, 0]));
        assert_eq!(show(&fmt(0x2A0C, 0), n("0")), "");
    }

    #[test]
    fn dates_as_filemaker_shows_them() {
        // 7 March 2026, a Saturday.
        let day = calc::value::day_number(2026, 3, 7);
        let d = |format: u8, ds: u8, ms: u8| {
            let mut f = Format::new_default();
            f.date.format = format;
            f.date.day_style = ds;
            f.date.month_style = ms;
            f.date.separator = '/';
            show(&f, Value::Date(day))
        };
        assert_eq!(d(0, 0, 0), "=");
        assert_eq!(d(2, 1, 0), "07/3/26");
        assert_eq!(d(2, 2, 0), " 7/3/26");
        assert_eq!(d(7, 0, 1), "7/03/2026");
        assert_eq!(d(3, 0, 0), "7 mars 2026");
        let nov = Value::Date(calc::value::day_number(2026, 11, 28));
        let mut f4 = Format::new_default();
        f4.date.format = 4;
        assert_eq!(show(&f4, nov), "28 novembre 2026");
        assert_eq!(d(5, 0, 0), "sam. 7 mars 2026");
        assert_eq!(d(6, 0, 0), "samedi 7 mars 2026");
        let mut f = Format::new_default();
        f.date.format = 1;
        f.date.elements = [1, 2, 3, 4];
        f.date.separators = [" ", "/", "-", ".", "!"].map(String::from);
        assert_eq!(show(&f, Value::Date(day)), " 7/sam.-samedi.3!");
        f.date.elements = [5, 7, 9, 10];
        assert_eq!(show(&f, Value::Date(day)), " mars/26-T1.1er trimestre!");
    }

    #[test]
    fn times_as_filemaker_shows_them() {
        let t = |flags: u16, hs: u8, ms: u8| {
            let mut f = Format::new_default();
            f.number.flags = 0x4A0C;
            f.number.digits = 2;
            f.time.flags = flags;
            f.time.hour_style = hs;
            f.time.minsec_style = ms;
            show(&f, Value::Time("50709".parse().unwrap()))
        };
        assert_eq!(t(0x8F, 0, 0), "=");
        assert_eq!(t(0x83, 0, 1), "2:05 PM");
        assert_eq!(t(0x83, 1, 1), "02:05 PM");
        assert_eq!(t(0x83, 2, 1), " 2:05 PM");
        assert_eq!(t(0x87, 0, 0), "2:5:9,00 PM");
        assert_eq!(t(0x87, 0, 1), "2:05:09,00 PM");
        assert_eq!(t(0x81, 0, 1), "2 PM");
        assert_eq!(t(0x86, 0, 1), "845:09,00 PM");
    }
}
