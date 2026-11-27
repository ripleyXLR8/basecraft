//! Exact decimal arithmetic on top of `BigDecimal`.
//!
//! FileMaker keeps up to 400 significant digits. Addition, subtraction and
//! multiplication of exact numbers are exact. At the default precision,
//! division and the math functions keep two guard digits beyond the 16
//! decimals FileMaker shows: such *inexact* results are stored with exactly
//! [`INEXACT`] decimal places (the only numbers with that scale), shown and
//! compared rounded to 16 decimals, and they make the results of + - * they
//! take part in inexact too (oracle: `1/3*3` shows 1, `(1/3)*10^20` shows
//! 33333333333333333300). With `SetPrecision` above 16, results are rounded
//! to that many decimals and exact.

use bigdecimal::num_bigint::BigInt;
use bigdecimal::{BigDecimal, RoundingMode, Signed, ToPrimitive, Zero};
use std::str::FromStr;

pub type Num = BigDecimal;

/// Default number of decimal places for division and math functions.
pub const DEFAULT_SCALE: i64 = 16;
/// Scale of inexact results at the default precision.
pub const INEXACT: i64 = DEFAULT_SCALE + 2;
const MAX_DIGITS: u64 = 400;

fn scale_of(x: &Num) -> i64 {
    x.as_bigint_and_exponent().1
}

/// Whether `x` is an inexact result (see the module docs).
pub fn is_inexact(x: &Num) -> bool {
    scale_of(x) == INEXACT
}

/// Marks `x` inexact: rounded to [`INEXACT`] places, scale kept.
pub fn inexact(x: &Num) -> Num {
    x.with_scale_round(INEXACT, RoundingMode::HalfUp)
}

/// The value FileMaker shows and compares: inexact results rounded to 16
/// decimals.
pub fn shown(x: &Num) -> Num {
    if is_inexact(x) {
        x.with_scale_round(DEFAULT_SCALE, RoundingMode::HalfUp)
            .normalized()
    } else {
        x.clone()
    }
}

/// The result of `r = a op b`: exact when both are; otherwise inexact, or
/// rounded to the finer exact operand (`1/3 + 0.1234567890123456789` keeps
/// its 19 decimals).
fn combine(r: Num, a: &Num, b: &Num) -> Num {
    let places = match (is_inexact(a), is_inexact(b)) {
        (false, false) => return exact(r),
        (true, true) => INEXACT,
        (true, false) => scale_of(b).max(INEXACT),
        (false, true) => scale_of(a).max(INEXACT),
    };
    if places == INEXACT {
        inexact(&r)
    } else {
        fix(r.with_scale_round(places, RoundingMode::HalfUp))
    }
}

/// A sum that is zero at the shown precision is zero: FileMaker gives 0
/// for `(1/3) * 3 - 1`.
fn snap(r: Num) -> Num {
    if is_inexact(&r) && shown(&r).is_zero() {
        int(0)
    } else {
        r
    }
}

pub fn add(a: &Num, b: &Num) -> Num {
    snap(combine(a + b, a, b))
}

pub fn sub(a: &Num, b: &Num) -> Num {
    snap(combine(a - b, a, b))
}

/// a / b to `scale` decimals for intermediate results, with the final
/// rounding of [`finish`].
pub fn quotient(a: &Num, b: &Num, scale: i64) -> Option<Num> {
    Some(finish(&div_places(a, b, scale + 4)?, scale))
}

pub fn mul(a: &Num, b: &Num) -> Num {
    combine(a * b, a, b)
}

/// Final rounding of a math function result: inexact at the default
/// precision, else `scale` decimals.
pub fn finish(x: &Num, scale: i64) -> Num {
    if scale == DEFAULT_SCALE {
        inexact(x)
    } else {
        round(x, scale + 2)
    }
}

pub fn int(i: i64) -> Num {
    Num::from(i)
}

pub fn parse(s: &str) -> Option<Num> {
    let x = Num::from_str(s).ok()?;
    // Beyond 400 digits either side of the point is out of FileMaker's range.
    let (_, scale) = x.as_bigint_and_exponent();
    (scale.abs() <= 800).then(|| exact(x))
}

/// Normalizes and caps a result at 400 significant digits. Inexact numbers
/// keep their scale; an exact number never ends up with that scale.
pub fn fix(x: Num) -> Num {
    if is_inexact(&x) && x.digits() <= MAX_DIGITS {
        return x;
    }
    exact(x)
}

/// `x` as an exact number: normalized, capped, never at the inexact scale.
pub fn exact(x: Num) -> Num {
    let x = if x.digits() > MAX_DIGITS {
        x.with_precision_round(
            std::num::NonZeroU64::new(MAX_DIGITS).unwrap(),
            RoundingMode::HalfUp,
        )
    } else {
        x
    };
    let x = x.normalized();
    if scale_of(&x) == INEXACT {
        x.with_scale(INEXACT + 1)
    } else {
        x
    }
}

pub fn round(x: &Num, places: i64) -> Num {
    fix(x.with_scale_round(places, RoundingMode::HalfUp))
}

pub fn trunc(x: &Num, places: i64) -> Num {
    fix(x.with_scale_round(places, RoundingMode::Down))
}

pub fn floor(x: &Num) -> Num {
    fix(x.with_scale_round(0, RoundingMode::Floor))
}

pub fn ceil(x: &Num) -> Num {
    fix(x.with_scale_round(0, RoundingMode::Ceiling))
}

/// Brings two decimals to integers on a common scale.
fn common(a: &Num, b: &Num) -> (BigInt, BigInt) {
    let (ma, sa) = a.as_bigint_and_exponent();
    let (mb, sb) = b.as_bigint_and_exponent();
    let s = sa.max(sb);
    (ma * pow10((s - sa) as u32), mb * pow10((s - sb) as u32))
}

fn pow10(e: u32) -> BigInt {
    BigInt::from(10u8).pow(e)
}

/// a / b. At the default precision the quotient is inexact, unless an
/// operand has more decimals than [`INEXACT`]: then it is rounded to those
/// (`0.1234567890123456789 / 1` keeps 19 decimals). Otherwise rounded half
/// away from zero to `scale` decimal places.
pub fn div(a: &Num, b: &Num, scale: i64) -> Option<Num> {
    if scale == DEFAULT_SCALE {
        let exact = |x: &Num| if is_inexact(x) { 0 } else { scale_of(x) };
        let places = exact(a).max(exact(b)).max(INEXACT);
        let q = div_places(a, b, places)?;
        return Some(if places == INEXACT {
            inexact(&q)
        } else {
            fix(q)
        });
    }
    // Two guard digits; SetPrecision rounds its result to `scale`.
    div_places(a, b, scale + 2)
}

fn div_places(a: &Num, b: &Num, scale: i64) -> Option<Num> {
    if b.is_zero() {
        return None;
    }
    let (ia, ib) = common(a, b);
    let shift = scale.max(0) as u32;
    let n = ia * pow10(shift);
    let (q, r) = (&n / &ib, &n % &ib);
    let q = if (r.abs() * 2u8) >= ib.abs() {
        if n.is_negative() != ib.is_negative() {
            q - 1
        } else {
            q + 1
        }
    } else {
        q
    };
    let x = Num::new(q, shift as i64);
    Some(if scale < 0 { round(&x, scale) } else { fix(x) })
}

/// floor(a / b), exact.
pub fn floor_div(a: &Num, b: &Num) -> Option<Num> {
    if b.is_zero() {
        return None;
    }
    let (ia, ib) = common(a, b);
    let (q, r) = (&ia / &ib, &ia % &ib);
    let q = if !r.is_zero() && (r.is_negative() != ib.is_negative()) {
        q - 1
    } else {
        q
    };
    Some(Num::from(q))
}

/// FileMaker Mod: the result has the sign of the divisor.
pub fn modulo(a: &Num, b: &Num) -> Option<Num> {
    let q = floor_div(a, b)?;
    Some(fix(a - b * q))
}

/// The nearest f64.
// ponytail: FileMaker's trigonometry differs from this conversion plus the
// platform libm in the last digit for about 1 argument in 5 (oracle:
// Sin ( Pi / 6 ) = .4999999999999999 where we get .5); no simple decimal
// -> double rule reproduces all of them.
pub fn to_f64(x: &Num) -> f64 {
    plain(x).parse().unwrap_or(f64::NAN)
}

/// A double result (trigonometry) as a number: at the default precision its
/// exact binary value to 20 significant digits, cut to the inexact scale.
pub fn from_f64(f: f64, scale: i64) -> Option<Num> {
    if !f.is_finite() {
        return None;
    }
    // Rust's Display for f64 never uses an exponent and prints the shortest
    // round-tripping digits.
    // The exact binary value to 20 significant digits, cut (not rounded) to
    // the inexact scale: Sin ( 1 ) is .841470984807896504 in FileMaker.
    let x = Num::try_from(f)
        .ok()?
        .with_precision_round(std::num::NonZeroU64::new(20).unwrap(), RoundingMode::HalfUp);
    Some(if scale == DEFAULT_SCALE {
        x.with_scale_round(INEXACT, RoundingMode::Down)
    } else {
        round(&x, scale)
    })
}

pub fn sqrt(x: &Num, scale: i64) -> Option<Num> {
    if x.is_negative() {
        return None;
    }
    let guard = scale.max(0) + 3;
    let (m, s) = x.as_bigint_and_exponent();
    let e = 2 * guard - s;
    let n = if e >= 0 {
        m * pow10(e as u32)
    } else {
        m / pow10((-e) as u32)
    };
    Some(finish(&Num::new(n.sqrt(), guard), scale))
}

/// Digits needed left of the decimal point, roughly, for exp(x).
fn exp_int_digits(x: &Num) -> i64 {
    (to_f64(x).max(0.0) * std::f64::consts::LOG10_E) as i64 + 1
}

pub fn exp(x: &Num, scale: i64) -> Option<Num> {
    // e^925 has more than 400 digits; e^-925 rounds to 0 at any precision.
    match to_f64(x) {
        f if f > 925.0 => return None,
        f if f < -925.0 => return Some(int(0)),
        _ => {}
    }
    if x.is_negative() {
        let w = scale + 10 + exp_int_digits(&-x);
        return quotient(&int(1), &exp(&-x, w)?, scale);
    }
    let mut k = 0u32;
    let mut r = x.clone();
    while r > Num::from_str("0.5").unwrap() {
        r = &r * Num::from_str("0.5").unwrap(); // exact, unlike `/`
        k += 1;
    }
    let w = scale + 10 + exp_int_digits(x) + k as i64;
    let r = round(&r, w);
    let mut sum = int(1);
    let mut term = int(1);
    let mut i = 1i64;
    loop {
        term = round(&(&term * &r), w);
        term = div(&term, &int(i), w)?;
        if term.is_zero() {
            break;
        }
        sum += &term;
        i += 1;
    }
    for _ in 0..k {
        sum = round(&(&sum * &sum), w);
    }
    Some(finish(&sum, scale))
}

pub fn ln(x: &Num, scale: i64) -> Option<Num> {
    if !x.is_positive() {
        return None;
    }
    // ln(x) = 2^j * ln(x^(1/2^j)); square roots bring the argument near 1,
    // where atanh's series converges quickly.
    let mut w = scale + 30;
    let mut y = x.clone();
    let mut j = 0u32;
    let one = int(1);
    let near = Num::from_str("0.01").unwrap();
    while (&y - &one).abs() > near {
        y = sqrt(&y, w)?;
        j += 1;
        if j > 2000 {
            return None;
        }
    }
    w += j as i64 / 3;
    let z = div(&(&y - &one), &(&y + &one), w)?;
    let z2 = round(&(&z * &z), w);
    let mut sum = z.clone();
    let mut pow = z;
    let mut k = 3i64;
    loop {
        pow = round(&(&pow * &z2), w);
        let term = div(&pow, &int(k), w)?;
        if term.is_zero() {
            break;
        }
        sum += term;
        k += 2;
    }
    let factor = BigInt::from(2u8).pow(j + 1);
    Some(finish(&(sum * Num::from(factor)), scale))
}

/// Pi by Machin's formula: 16 atan(1/5) - 4 atan(1/239).
pub fn pi(scale: i64) -> Num {
    let w = (scale.max(0) + 10) as u32;
    let unit = pow10(w);
    let atan_inv = |n: u32| {
        let n2 = BigInt::from(n * n);
        let mut p = &unit / BigInt::from(n);
        let mut sum = BigInt::zero();
        let mut k = 1u32;
        let mut neg = false;
        while !p.is_zero() {
            let t = &p / BigInt::from(k);
            if neg {
                sum -= t
            } else {
                sum += t
            }
            p /= &n2;
            k += 2;
            neg = !neg;
        }
        sum
    };
    let v = atan_inv(5) * 16 - atan_inv(239) * 4;
    finish(&Num::new(v, w as i64), scale)
}

/// x^n by squaring, exact (capped at 400 digits).
pub fn powi(x: &Num, mut n: u64) -> Option<Num> {
    let mut acc = int(1);
    let mut base = x.clone();
    while n > 0 {
        if n & 1 == 1 {
            acc = cap(&acc * &base);
        }
        n >>= 1;
        if n > 0 {
            base = cap(&base * &base);
        }
    }
    Some(acc)
}

fn cap(x: Num) -> Num {
    if x.digits() > MAX_DIGITS + 50 {
        x.with_precision_round(
            std::num::NonZeroU64::new(MAX_DIGITS + 50).unwrap(),
            RoundingMode::HalfUp,
        )
    } else {
        x
    }
}

pub fn pow(x: &Num, y: &Num, scale: i64) -> Option<Num> {
    if y.is_integer() {
        let e = y.to_i64()?;
        if e.unsigned_abs() <= 4096 {
            if x.is_zero() && e < 0 {
                return None;
            }
            let acc = powi(x, e.unsigned_abs())?;
            if acc.digits() > MAX_DIGITS && !is_inexact(x) {
                return None;
            }
            return if e < 0 {
                div(&int(1), &acc, scale)
            } else if is_inexact(x) {
                Some(inexact(&acc))
            } else {
                Some(acc)
            };
        }
    }
    if x.is_zero() {
        return if y.is_positive() { Some(int(0)) } else { None };
    }
    let odd_int = y.is_integer() && (y % int(2)).abs() == int(1);
    if x.is_negative() && !y.is_integer() {
        return None;
    }
    let w = scale + 10 + exp_int_digits(&(y * ln(&x.abs(), 20)?));
    let r = exp(&(y * ln(&x.abs(), w)?), scale)?;
    Some(if x.is_negative() && odd_int { -r } else { r })
}

/// Plain decimal text: no exponent, trailing zeros trimmed, leading 0 before
/// the decimal point. Formula text uses this form.
pub fn plain(x: &Num) -> String {
    let x = x.normalized();
    if x.is_zero() {
        return "0".into();
    }
    let (m, s) = x.as_bigint_and_exponent();
    let neg = m.is_negative();
    let digits = m.abs().to_string();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if s <= 0 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (-s) as usize));
    } else {
        let s = s as usize;
        if digits.len() > s {
            let (a, b) = digits.split_at(digits.len() - s);
            out.push_str(a);
            out.push('.');
            out.push_str(b);
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', s - digits.len()));
            out.push_str(&digits);
        }
    }
    out
}

/// A number as FileMaker shows it: inexact results rounded to 16 decimals,
/// no leading zero (`.5`), and scientific notation (`1.0E+9`, `1.5E-10`,
/// `1.0000000005E+9`) when the whole part ends with nine zeros or more, or
/// the first significant digit is below 10^-9.
pub fn format(x: &Num) -> String {
    let x = shown(x).normalized();
    if x.is_zero() {
        return "0".into();
    }
    let (m, s) = x.as_bigint_and_exponent();
    let sign = if m.is_negative() { "-" } else { "" };
    let digits = m.abs().to_string();
    let top = digits.len() as i64 - 1 - s; // exponent of the first digit
    let whole_len = digits.len() as i64 - s.max(0);
    let whole_zeros = if s <= 0 {
        -s
    } else if whole_len > 0 {
        let whole = &digits[..whole_len as usize];
        (whole.len() - whole.trim_end_matches('0').len()) as i64
    } else {
        0
    };
    if whole_zeros >= 9 || top <= -10 {
        let rest = if digits.len() > 1 { &digits[1..] } else { "0" };
        let e = if top < 0 {
            format!("-{}", -top)
        } else {
            format!("+{top}")
        };
        return format!("{sign}{}.{rest}E{e}", &digits[..1]);
    }
    let p = plain(&x);
    match p.strip_prefix("0.").or_else(|| p.strip_prefix("-0.")) {
        Some(frac) => format!("{sign}.{frac}"),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Num {
        parse(s).unwrap()
    }

    #[test]
    fn arithmetic() {
        let third = div(&n("1"), &n("3"), 16).unwrap();
        assert!(is_inexact(&third));
        assert_eq!(format(&third), ".3333333333333333");
        assert_eq!(format(&mul(&third, &n("3"))), "1");
        assert_eq!(
            format(&div(&n("2"), &n("3"), 16).unwrap()),
            ".6666666666666667"
        );
        assert_eq!(
            format(&div(&n("-2"), &n("3"), 16).unwrap()),
            "-.6666666666666667"
        );
        assert_eq!(
            plain(&round(&div(&n("5"), &n("9"), 30).unwrap(), 30)),
            "0.555555555555555555555555555556"
        );
        assert_eq!(format(&n("100")), "100");
        assert_eq!(format(&n("1.500")), "1.5");
        assert_eq!(format(&n("-0.05")), "-.05");
        assert_eq!(format(&add(&n("0.1"), &n("0.2"))), ".3");
        assert_eq!(format(&n("1000000000")), "1.0E+9");
        assert_eq!(format(&n("123000000000")), "1.23E+11");
        assert_eq!(format(&n("0.0000000001")), "1.0E-10");
        assert_eq!(format(&n("0.000000001")), ".000000001");
        assert_eq!(format(&n("0.000000000000000001")), "1.0E-18");
        assert_eq!(plain(&modulo(&n("-7"), &n("3")).unwrap()), "2");
        assert_eq!(plain(&modulo(&n("7"), &n("-3")).unwrap()), "-2");
        assert_eq!(plain(&modulo(&n("5.5"), &n("2")).unwrap()), "1.5");
        assert_eq!(plain(&floor_div(&n("-7"), &n("3")).unwrap()), "-3");
    }

    #[test]
    fn transcendental() {
        assert_eq!(format(&sqrt(&n("4"), 16).unwrap()), "2");
        assert_eq!(format(&sqrt(&n("2"), 16).unwrap()), "1.414213562373095");
        assert_eq!(format(&pi(16)), "3.1415926535897932");
        assert_eq!(
            plain(&round(&pi(50), 50)),
            "3.14159265358979323846264338327950288419716939937511"
        );
        assert_eq!(format(&exp(&n("1"), 16).unwrap()), "2.7182818284590452");
        assert_eq!(
            plain(&round(&exp(&n("50"), 25).unwrap(), 25)),
            "5184705528587072464087.4533229334853848274691006"
        );
        assert_eq!(format(&ln(&n("1"), 16).unwrap()), "0");
        assert_eq!(format(&ln(&exp(&n("2"), 40).unwrap(), 16).unwrap()), "2");
        assert_eq!(format(&ln(&n("10"), 16).unwrap()), "2.3025850929940457");
        assert_eq!(format(&pow(&n("2"), &n("10"), 16).unwrap()), "1024");
        assert_eq!(format(&pow(&n("2"), &n("-1"), 16).unwrap()), ".5");
        assert_eq!(format(&pow(&n("4"), &n("0.5"), 16).unwrap()), "2");
        assert!(pow(&n("-4"), &n("0.5"), 16).is_none());
    }
}
