//! Date, time and timestamp functions, plus the built-in `Get()` clock flags.

use super::{int, number};
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num::{self, Num};
use crate::value::{self, Value, civil, day_number, day_of_week as dow, make_ts, valid_day};

fn bad_date(what: &str) -> EvalError {
    EvalError::new(code::DATE_INVALID, format!("{what}: invalid date"))
}

fn bad_time(what: &str) -> EvalError {
    EvalError::new(code::TIME_INVALID, format!("{what}: invalid time"))
}

/// Date argument: empty (or a time, which has no date) stays empty (None),
/// invalid is error 500.
fn date_arg(v: &Value, what: &str) -> Result<Option<i64>, EvalError> {
    if v.is_empty() || matches!(v, Value::Time(_)) {
        return Ok(None);
    }
    v.to_date()
        .filter(|d| valid_day(*d))
        .map(Some)
        .ok_or_else(|| bad_date(what))
}

fn time_arg(v: &Value, what: &str) -> Result<Option<Num>, EvalError> {
    if v.is_empty() {
        return Ok(None);
    }
    v.to_time().map(Some).ok_or_else(|| bad_time(what))
}

pub(crate) fn on_date(a: &[Value], what: &str, f: impl FnOnce(i64) -> Value) -> R {
    Ok(date_arg(&a[0], what)?.map(f).unwrap_or_else(Value::empty))
}

fn on_time(a: &[Value], what: &str, f: impl FnOnce(Num) -> Value) -> R {
    Ok(time_arg(&a[0], what)?.map(f).unwrap_or_else(Value::empty))
}

pub fn date(_: &mut Evaluator, a: &[Value]) -> R {
    if a[0].is_empty() || a[2].is_empty() {
        return Err(bad_date("Date"));
    }
    let (y, m, d) = (int(a, 2)?, int(a, 0)?, int(a, 1)?);
    if y.abs() > 100_000 || m.abs() > 1_000_000 || d.abs() > 100_000_000 {
        return Err(bad_date("Date"));
    }
    let d = day_number(y, m, d);
    if valid_day(d) {
        Ok(Value::Date(d))
    } else {
        Err(bad_date("Date"))
    }
}

pub fn time(_: &mut Evaluator, a: &[Value]) -> R {
    let h = num::mul(&number(a, 0), &num::int(3600));
    let m = num::mul(&number(a, 1), &num::int(60));
    Ok(Value::Time(num::add(&num::add(&h, &m), &number(a, 2))))
}

pub fn timestamp(_: &mut Evaluator, a: &[Value]) -> R {
    if a[1].is_empty() && a[0].is_empty() {
        return Ok(Value::empty());
    }
    let d = date_arg(&a[0], "Timestamp")?.ok_or_else(|| bad_date("Timestamp"))?;
    let Some(t) = time_arg(&a[1], "Timestamp")? else {
        return Ok(Value::empty());
    };
    let ts = make_ts(d, &t);
    if !valid_day(value::ts_date(&ts)) {
        return Err(bad_date("Timestamp"));
    }
    Ok(Value::Timestamp(ts))
}
pub fn year(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "Year", |d| Value::int(civil(d).0))
}

pub fn month(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "Month", |d| Value::int(civil(d).1))
}

pub fn day(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "Day", |d| Value::int(civil(d).2))
}

pub fn day_of_week(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "DayOfWeek", |d| Value::int(dow(d)))
}

fn doy(d: i64) -> i64 {
    d - day_number(civil(d).0, 1, 1) + 1
}

pub fn day_of_year(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "DayOfYear", |d| Value::int(doy(d)))
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const DAYS_J: [&str; 7] = [
    "日曜日",
    "月曜日",
    "火曜日",
    "水曜日",
    "木曜日",
    "金曜日",
    "土曜日",
];
const MONTHS: [&str; 12] = [
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

pub fn day_name(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "DayName", |d| Value::text(DAYS[dow(d) as usize - 1]))
}

pub fn day_name_j(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "DayNameJ", |d| Value::text(DAYS_J[dow(d) as usize - 1]))
}

pub fn month_name(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "MonthName", |d| {
        Value::text(MONTHS[civil(d).1 as usize - 1])
    })
}

/// Week 1 holds January 1; weeks start on Sunday.
pub fn week_of_year(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "WeekOfYear", |d| {
        let jan1 = day_number(civil(d).0, 1, 1);
        Value::int((doy(d) - 1 + dow(jan1) - 1) / 7 + 1)
    })
}

/// Week 1 is the first week with at least four days in the new year; weeks
/// start on `startingDay` (1 = Sunday; FileMaker clamps it to 1..7).
pub fn week_of_year_fiscal(_: &mut Evaluator, a: &[Value]) -> R {
    let start = int(a, 1)?.clamp(1, 7);
    on_date(a, "WeekOfYearFiscal", |d| {
        let week_start = d - (dow(d) - start).rem_euclid(7);
        let mid = week_start + 3;
        Value::int((mid - day_number(civil(mid).0, 1, 1)) / 7 + 1)
    })
}

/// (hours, minutes, seconds) of a duration, each truncated toward zero
/// (Minute ( -5400 ) = -30); seconds keep 6 decimals.
fn hms(t: &Num) -> (Num, Num, Num) {
    let t = num::shown(t);
    let h = num::trunc(&num::div(&t, &num::int(3600), 30).unwrap_or_default(), 0);
    let rest = &t - &h * num::int(3600);
    let m = num::trunc(&num::div(&rest, &num::int(60), 30).unwrap_or_default(), 0);
    let s = num::trunc(&(rest - &m * num::int(60)), 6);
    (h, m, s)
}

pub fn hour(_: &mut Evaluator, a: &[Value]) -> R {
    on_time(a, "Hour", |t| Value::num(hms(&t).0))
}

pub fn minute(_: &mut Evaluator, a: &[Value]) -> R {
    on_time(a, "Minute", |t| Value::num(hms(&t).1))
}

pub fn seconds(_: &mut Evaluator, a: &[Value]) -> R {
    on_time(a, "Seconds", |t| Value::num(hms(&t).2))
}

pub fn get_as_date(_: &mut Evaluator, a: &[Value]) -> R {
    on_date(a, "GetAsDate", Value::Date)
}

pub fn get_as_time(_: &mut Evaluator, a: &[Value]) -> R {
    on_time(a, "GetAsTime", Value::Time)
}

pub fn get_as_timestamp(_: &mut Evaluator, a: &[Value]) -> R {
    let v = &a[0];
    let t = match v {
        _ if v.is_empty() => return Ok(Value::empty()),
        Value::Timestamp(t) | Value::Number(t) | Value::Time(t) => Some(t.clone()),
        Value::Date(d) => Some(make_ts(*d, &num::int(0))),
        Value::Text(s) | Value::Json { text: s, .. } => value::parse_timestamp(s),
        Value::Container(_) => None,
    };
    t.filter(|t| valid_day(value::ts_date(t)))
        .map(Value::Timestamp)
        .ok_or_else(|| bad_date("GetAsTimestamp"))
}
// ---- Get() flags that need no database -----------------------------------------

fn now_utc_micros() -> i128 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let epoch = (day_number(1970, 1, 1) - 1) as i128 * 86_400_000_000;
    epoch + since.as_micros() as i128
}

fn uuid() -> String {
    use super::number::random_u64;
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&random_u64().to_le_bytes());
    b[8..].copy_from_slice(&random_u64().to_le_bytes());
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

thread_local! {
    static USER_NAME: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// `Get ( UserName )`: the user name of the application's preferences
/// (FileMaker's Preferences › General › User name), set by the host for
/// its thread. `None` gives back the system account's name.
pub fn set_user_name(name: Option<String>) {
    USER_NAME.with(|u| *u.borrow_mut() = name);
}

fn user_name() -> String {
    USER_NAME
        .with(|u| u.borrow().clone())
        .or_else(|| std::env::var("USERNAME").ok())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_default()
}

/// Clock and platform flags. The clock is UTC: the engine has no time zone
/// data.
// ponytail: CurrentDate/Time are UTC; hosts should answer them through
// Context::get with local time.
pub fn builtin_get(flag: &str) -> Option<Value> {
    let micros = now_utc_micros();
    let secs = num::int((micros / 1_000_000) as i64);
    Some(match flag {
        "CurrentDate" => Value::Date(value::ts_date(&secs)),
        "CurrentTime" => Value::Time(value::ts_time(&secs)),
        "CurrentTimestamp" | "CurrentHostTimestamp" => Value::Timestamp(secs),
        "CurrentTimeUTCMilliseconds" => Value::int((micros / 1000) as i64),
        "CurrentTimeUTCMicroseconds" => Value::int(micros as i64),
        "UUID" => Value::Text(uuid()),
        "CalculationRepetitionNumber" => Value::int(1),
        "UserName" => Value::Text(user_name()),
        "SystemPlatform" => Value::int(match std::env::consts::OS {
            "macos" => 1,
            "windows" => -2,
            "ios" => 3,
            _ => 5,
        }),
        "ApplicationArchitecture" => Value::text(match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86" => "i386",
            other => other,
        }),
        _ => return None,
    })
}
