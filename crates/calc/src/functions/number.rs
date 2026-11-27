//! Number functions.

use super::{int, number};
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num::{self, Num};
use crate::value::Value;
use bigdecimal::num_bigint::BigInt;
use bigdecimal::{Signed, ToPrimitive, Zero};

fn bad(what: &str) -> EvalError {
    EvalError::new(code::NUMBER_DOMAIN, format!("{what}: invalid argument"))
}

/// Applies `f` to the first argument; empty input gives empty, text without
/// a number counts as 0 (Abs ( "x" ) = 0).
fn on_num(a: &[Value], f: impl FnOnce(Num) -> Option<Num>, what: &str) -> R {
    if a[0].is_empty() {
        return Ok(Value::empty());
    }
    f(a[0].num_or_zero())
        .map(Value::num)
        .ok_or_else(|| bad(what))
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
    on_num(a, |n| Some(num::trunc(&num::shown(&n), 0)), "Int")
}

pub fn floor(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(num::floor(&num::shown(&n))), "Floor")
}

pub fn ceiling(_: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| Some(num::ceil(&num::shown(&n))), "Ceiling")
}

/// An inexact number keeps its guard digits: Round ( 1/3 ; 17 ) * 3 = 1.
pub fn round(_: &mut Evaluator, a: &[Value]) -> R {
    let p = int(a, 1)?.clamp(-400, 400);
    on_num(
        a,
        |n| {
            Some(if num::is_inexact(&n) {
                num::inexact(&num::round(&n, p.min(num::INEXACT)))
            } else {
                num::round(&n, p)
            })
        },
        "Round",
    )
}

/// Works on the shown value: Truncate ( 1/3 ; 20 ) = .3333333333333333.
pub fn truncate(_: &mut Evaluator, a: &[Value]) -> R {
    let p = int(a, 1)?.clamp(-400, 400);
    on_num(a, |n| Some(num::trunc(&num::shown(&n), p)), "Truncate")
}

fn divide_by_zero() -> EvalError {
    EvalError::new(code::DIVIDE_BY_ZERO, "division by zero")
}

/// Mod ( n ; 0 ) is n, but an empty divisor is a division by zero.
pub fn modulo(_: &mut Evaluator, a: &[Value]) -> R {
    if a[1].is_empty() {
        return Err(divide_by_zero());
    }
    let d = number(a, 1);
    if d.is_zero() {
        return Ok(Value::num(number(a, 0)));
    }
    let n = number(a, 0);
    let r = num::modulo(&n, &d).ok_or_else(divide_by_zero)?;
    Ok(Value::num(if num::is_inexact(&n) || num::is_inexact(&d) {
        num::inexact(&r)
    } else {
        r
    }))
}

pub fn div(_: &mut Evaluator, a: &[Value]) -> R {
    let d = number(a, 1);
    if a[0].is_empty() {
        return Ok(Value::empty());
    }
    let n = number(a, 0);
    num::floor_div(&num::shown(&n), &num::shown(&d))
        .map(Value::num)
        .ok_or_else(divide_by_zero)
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
    num::quotient(&num::ln(n, w)?, &num::ln(&num::int(base), w)?, scale)
}

pub fn log(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| log_base(&n, 10, ev.scale), "Log")
}

pub fn lg(ev: &mut Evaluator, a: &[Value]) -> R {
    on_num(a, |n| log_base(&n, 2, ev.scale), "Lg")
}

/// Trigonometry runs on f64, as in FileMaker (Sin ( Pi ) = 1.0E-16).
/// Outside the domain (Asin ( 2 )) the number cannot be evaluated: 1218.
fn trig(ev: &Evaluator, a: &[Value], f: fn(f64) -> f64, what: &str) -> R {
    if a[0].is_empty() {
        return Ok(Value::empty());
    }
    num::from_f64(f(num::to_f64(&a[0].num_or_zero())), ev.scale)
        .map(Value::Number)
        .ok_or_else(|| {
            EvalError::new(
                code::CANNOT_EVALUATE_NUMBER,
                format!("{what}: out of range"),
            )
        })
}

pub fn sin(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::sin, "Sin")
}

pub fn cos(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::cos, "Cos")
}

pub fn tan(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::tan, "Tan")
}

pub fn asin(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::asin, "Asin")
}

pub fn acos(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::acos, "Acos")
}

pub fn atan(ev: &mut Evaluator, a: &[Value]) -> R {
    trig(ev, a, f64::atan, "Atan")
}

pub fn pi(ev: &mut Evaluator, _: &[Value]) -> R {
    Ok(Value::Number(num::pi(ev.scale)))
}

pub fn degrees(ev: &mut Evaluator, a: &[Value]) -> R {
    let w = ev.scale + 5;
    on_num(
        a,
        |n| {
            Some(num::finish(
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
        |n| {
            Some(num::finish(
                &num::div(&(n * num::pi(w)), &num::int(180), w)?,
                ev.scale,
            ))
        },
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

/// Non-negative whole number argument; anything else is error 14.
fn whole(a: &[Value], i: usize, what: &str) -> Result<i64, EvalError> {
    let n = num::shown(&number(a, i));
    if !n.is_integer() || n.is_negative() || n > num::int(10_000) {
        return Err(bad(what));
    }
    Ok(n.to_i64().unwrap_or(0))
}

pub fn factorial(_: &mut Evaluator, a: &[Value]) -> R {
    let n = whole(a, 0, "Factorial")?;
    let k = if a.len() > 1 {
        whole(a, 1, "Factorial")?
    } else {
        n
    };
    if k > n {
        return Err(bad("Factorial"));
    }
    Ok(Value::num(Num::from(falling(n, k))))
}

pub fn combination(_: &mut Evaluator, a: &[Value]) -> R {
    let (n, k) = (whole(a, 0, "Combination")?, whole(a, 1, "Combination")?);
    if k > n {
        return Err(bad("Combination"));
    }
    let k = k.min(n - k);
    Ok(Value::num(Num::from(falling(n, k) / falling(k, k))))
}

/// (1 + rate)^periods, exact for whole periods.
fn growth(rate: &Num, periods: &Num, scale: i64) -> Result<Num, EvalError> {
    let base = num::int(1) + rate;
    if let Some(n) = periods.to_i64().filter(|_| periods.is_integer()) {
        let p = num::powi(&base, n.unsigned_abs()).ok_or_else(|| bad("financial"))?;
        return if n < 0 {
            num::div(&num::int(1), &p, scale + 30).ok_or_else(divide_by_zero)
        } else {
            Ok(p)
        };
    }
    num::pow(&base, periods, scale + 30).ok_or_else(|| bad("financial"))
}

pub fn fv(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return Ok(Value::num(p * n));
    }
    let g = growth(&r, &n, ev.scale)?;
    let v = num::quotient(&(p * (g - num::int(1))), &r, ev.scale).ok_or_else(divide_by_zero)?;
    Ok(Value::Number(v))
}

pub fn pv(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return Ok(Value::num(p * n));
    }
    let g = growth(&r, &-n, ev.scale)?;
    let v = num::quotient(&(p * (num::int(1) - g)), &r, ev.scale).ok_or_else(divide_by_zero)?;
    Ok(Value::Number(v))
}

pub fn pmt(ev: &mut Evaluator, a: &[Value]) -> R {
    let (p, r, n) = (number(a, 0), number(a, 1), number(a, 2));
    if r.is_zero() {
        return num::div(&p, &n, ev.scale)
            .map(Value::Number)
            .ok_or_else(divide_by_zero);
    }
    let g = growth(&r, &-n, ev.scale)?;
    num::quotient(&(p * r), &(num::int(1) - g), ev.scale)
        .map(Value::Number)
        .ok_or_else(divide_by_zero)
}

/// red * 65536 + green * 256 + blue, unclamped (RGB ( 300 ; 0 ; 0 ) = 19660800).
pub fn rgb(_: &mut Evaluator, a: &[Value]) -> R {
    let c = |i, m| num::mul(&number(a, i), &num::int(m));
    Ok(Value::num(num::add(
        &num::add(&c(0, 65536), &c(1, 256)),
        &number(a, 2),
    )))
}
