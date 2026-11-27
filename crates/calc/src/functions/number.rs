//! Number functions.

use super::{int, number};
use crate::error::EvalError;
use crate::eval::{Evaluator, R};
use crate::num::{self, Num};
use crate::value::Value;
use bigdecimal::num_bigint::BigInt;
use bigdecimal::{Signed, ToPrimitive, Zero};

fn bad(what: &str) -> EvalError {
    EvalError::generic(format!("{what}: invalid argument"))
}

/// Applies `f` to the first argument; empty (non-numeric) input gives empty.
fn on_num(a: &[Value], f: impl FnOnce(Num) -> Option<Num>, what: &str) -> R {
    match a[0].to_num() {
        None => Ok(Value::empty()),
        Some(n) => f(n).map(Value::num).ok_or_else(|| bad(what)),
    }
}

pub fn abs(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(n.abs()), "Abs")
}

pub fn sign(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(
        a,
        |n| Some(num::int(n.signum().to_i64().unwrap_or(0))),
        "Sign",
    )
}

pub fn int_(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(num::trunc(&n, 0)), "Int")
}

pub fn floor(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(num::floor(&n)), "Floor")
}

pub fn ceiling(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(num::ceil(&n)), "Ceiling")
}

pub fn round(_: &mut Evaluator, a: &[Value]) -> R {
    let p = int(a, 1)?.clamp(-400, 400);
    on_num(a, |n| Some(num::round(&n, p)), "Round")
}

pub fn truncate(_: &mut Evaluator, a: &[Value]) -> R {
    let p = int(a, 1)?.clamp(-400, 400);
    on_num(a, |n| Some(num::trunc(&n, p)), "Truncate")
}

pub fn modulo(_: &mut Evaluator, a: &[Value]) -> R {
    let d = number(a, 1);
    on_num(a, |n| num::modulo(&n, &d), "Mod")
}

pub fn div(_: &mut Evaluator, a: &[Value]) -> R {
    let d = number(a, 1);
    on_num(a, |n| num::floor_div(&n, &d), "Div")
}

pub fn sqrt(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| num::sqrt(&n, ev.scale), "Sqrt")
}

pub fn exp(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| num::exp(&n, ev.scale), "Exp")
}

pub fn ln(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| num::ln(&n, ev.scale), "Ln")
}

fn log_base(n: &Num, base: i64, scale: i64) -> Option<Num> {
    let w = scale + 10;
    num::div(&num::ln(n, w)?, &num::ln(&num::int(base), w)?, scale)
}

pub fn log(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| log_base(&n, 10, ev.scale), "Log")
}

pub fn lg(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| log_base(&n, 2, ev.scale), "Lg")
}

fn trig(a: &[Value], f: fn(f64) -> f64, what: &str) -> R {
    on_num(
        a,
        |n| num::from_f64(f(num::to_f64(&n)), num::DEFAULT_SCALE),
        what,
    )
}

pub fn sin(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::sin, "Sin")
}

pub fn cos(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::cos, "Cos")
}

pub fn tan(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::tan, "Tan")
}

pub fn asin(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::asin, "Asin")
}

pub fn acos(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::acos, "Acos")
}

pub fn atan(_: &mut Evaluator, a: &[Value]) -> R {
    trig(a, f64::atan, "Atan")
}

pub fn pi(ev: &mut Evaluator, _: &[Value]) -> R {
    Ok(Value::Number(num::pi(ev.scale)))
}

pub fn degrees(ev: &mut Evaluator, a: &[Value]) -> R {
    let w = ev.scale + 5;
    on_num(
        a,
        |n| {
            Some(num::round(
                &num::div(&(n * num::int(180)), &num::pi(w), w)?,
                ev.scale,
            ))
        },
        "Degrees",
    )
}

pub fn radians(ev: &mut Evaluator, a: &[Value]) -> R {
    let w = ev.scale + 5;
    on_num(
        a,
        |n| num::div(&(n * num::pi(w)), &num::int(180), ev.scale),
        "Radians",
    )
}

/// A random u64 from std's per-process random hasher keys.
// ponytail: not cryptographic; fine for Random and Get(UUID).
pub fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.finish()
}

pub fn random(_: &mut Evaluator, _: &[Value]) -> R {
    let f = (random_u64() >> 11) as f64 / (1u64 << 53) as f64;
    Ok(Value::Number(num::from_f64(f, 20).unwrap_or_default()))
}

/// n * (n-1) * ... (k factors), exact.
fn falling(n: i64, k: i64) -> BigInt {
    (0..k).fold(BigInt::from(1u8), |acc, i| acc * (n - i))
}

pub fn factorial(_: &mut Evaluator, a: &[Value]) -> R {
    let n = int(a, 0)?;
    let k = if a.len() > 1 { int(a, 1)? } else { n };
    if !(0..=10_000).contains(&n) || k < 0 || k > n {
        return Err(bad("Factorial"));
    }
    Ok(Value::num(Num::from(falling(n, k))))
}

pub fn combination(_: &mut Evaluator, a: &[Value]) -> R {
    let (n, k) = (int(a, 0)?, int(a, 1)?);
    if !(0..=10_000).contains(&n) || k < 0 || k > n {
        return Err(bad("Combination"));
    }
    let k = k.min(n - k);
    Ok(Value::num(Num::from(falling(n, k) / falling(k, k))))
}

/// (1 + rate)^periods
fn growth(rate: &Num, periods: &Num, scale: i64) -> Result<Num, EvalError> {
    num::pow(&(num::int(1) + rate), periods, scale + 10).ok_or_else(|| bad("financial"))
}

pub fn fv(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return Ok(Value::num(p * n));
    }
    let g = growth(&r, &n, ev.scale)?;
    let v = num::div(&(p * (g - num::int(1))), &r, ev.scale).unwrap();
    Ok(Value::Number(v))
}

pub fn pv(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return Ok(Value::num(p * n));
    }
    let g = growth(&r, &-n, ev.scale)?;
    let v = num::div(&(p * (num::int(1) - g)), &r, ev.scale).unwrap();
    Ok(Value::Number(v))
}

pub fn pmt(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return num::div(&p, &n, ev.scale)
            .map(Value::Number)
            .ok_or_else(|| bad("PMT"));
    }
    let g = growth(&r, &-n, ev.scale)?;
    num::div(&(p * r), &(num::int(1) - g), ev.scale)
        .map(Value::Number)
        .ok_or_else(|| bad("PMT"))
}

pub fn rgb(_: &mut Evaluator, a: &[Value]) -> R {
    let c = |i| int(a, i).map(|v| v.clamp(0, 255));
    Ok(Value::int(c(0)? * 65536 + c(1)? * 256 + c(2)?))
}
