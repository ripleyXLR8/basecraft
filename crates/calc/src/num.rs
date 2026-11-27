//! Exact decimal arithmetic on top of `BigDecimal`.
//!
//! FileMaker keeps up to 400 significant digits. Addition, subtraction and
//! multiplication are exact; division and the transcendental functions round
//! half away from zero to `scale` digits right of the decimal point (16 by
//! default, up to 400 with `SetPrecision`).

use bigdecimal::num_bigint::BigInt;
use bigdecimal::{BigDecimal, RoundingMode, Signed, ToPrimitive, Zero};
use std::str::FromStr;

pub type Num = BigDecimal;

/// Default number of decimal places for division and math functions.
pub const DEFAULT_SCALE: i64 = 16;
const MAX_DIGITS: u64 = 400;

pub fn int(i: i64) -> Num {
    Num::from(i)
}

pub fn parse(s: &str) -> Option<Num> {
    let x = Num::from_str(s).ok()?;
    // Beyond 400 digits either side of the point is out of FileMaker's range.
    let (_, scale) = x.as_bigint_and_exponent();
    (scale.abs() <= 800).then(|| fix(x))
}

/// Normalizes and caps a result at 400 significant digits.
pub fn fix(x: Num) -> Num {
    let x = if x.digits() > MAX_DIGITS {
        x.with_precision_round(
            std::num::NonZeroU64::new(MAX_DIGITS).unwrap(),
            RoundingMode::HalfUp,
        )
    } else {
        x
    };
    x.normalized()
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

/// a / b rounded half away from zero to `scale` decimal places.
pub fn div(a: &Num, b: &Num, scale: i64) -> Option<Num> {
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

pub fn to_f64(x: &Num) -> f64 {
    x.to_f64().unwrap_or(f64::NAN)
}

// ponytail: f64 results carry ~16 significant digits; fine for trig, which
// FileMaker itself documents as not supporting extended precision.
pub fn from_f64(f: f64, scale: i64) -> Option<Num> {
    if !f.is_finite() {
        return None;
    }
    // Rust's Display for f64 never uses an exponent and prints the shortest
    // round-tripping digits.
    Some(round(&Num::from_str(&format!("{f}")).ok()?, scale))
}

pub fn sqrt(x: &Num, scale: i64) -> Option<Num> {
    if x.is_negative() {
        return None;
    }
    let guard = scale.max(0) + 1;
    let (m, s) = x.as_bigint_and_exponent();
    let e = 2 * guard - s;
    let n = if e >= 0 {
        m * pow10(e as u32)
    } else {
        m / pow10((-e) as u32)
    };
    Some(round(&Num::new(n.sqrt(), guard), scale))
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
        return div(&int(1), &exp(&-x, w)?, scale);
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
    Some(round(&sum, scale))
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
    Some(round(&(sum * Num::from(factor)), scale))
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
    round(&Num::new(v, w as i64), scale)
}

pub fn pow(x: &Num, y: &Num, scale: i64) -> Option<Num> {
    if y.is_integer() {
        let e = y.to_i64()?;
        if e.unsigned_abs() <= 4096 {
            if x.is_zero() && e < 0 {
                return None;
            }
            let mut acc = int(1);
            let mut base = x.clone();
            let mut n = e.unsigned_abs();
            while n > 0 {
                if n & 1 == 1 {
                    acc = fix(&acc * &base);
                }
                base = fix(&base * &base);
                n >>= 1;
            }
            return if e < 0 {
                div(&int(1), &acc, scale)
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
/// the decimal point.
pub fn format(x: &Num) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Num {
        parse(s).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(
            format(&div(&n("1"), &n("3"), 16).unwrap()),
            "0.3333333333333333"
        );
        assert_eq!(
            format(&div(&n("2"), &n("3"), 16).unwrap()),
            "0.6666666666666667"
        );
        assert_eq!(
            format(&div(&n("-2"), &n("3"), 16).unwrap()),
            "-0.6666666666666667"
        );
        assert_eq!(
            format(&div(&n("5"), &n("9"), 30).unwrap()),
            "0.555555555555555555555555555556"
        );
        assert_eq!(format(&n("100")), "100");
        assert_eq!(format(&n("1.500")), "1.5");
        assert_eq!(format(&n("-0.05")), "-0.05");
        assert_eq!(format(&(n("0.1") + n("0.2"))), "0.3");
        assert_eq!(format(&modulo(&n("-7"), &n("3")).unwrap()), "2");
        assert_eq!(format(&modulo(&n("7"), &n("-3")).unwrap()), "-2");
        assert_eq!(format(&modulo(&n("5.5"), &n("2")).unwrap()), "1.5");
        assert_eq!(format(&floor_div(&n("-7"), &n("3")).unwrap()), "-3");
    }

    #[test]
    fn transcendental() {
        assert_eq!(format(&sqrt(&n("4"), 16).unwrap()), "2");
        assert_eq!(format(&sqrt(&n("2"), 16).unwrap()), "1.414213562373095");
        assert_eq!(format(&pi(16)), "3.1415926535897932");
        assert_eq!(
            format(&pi(50)),
            "3.14159265358979323846264338327950288419716939937511"
        );
        assert_eq!(format(&exp(&n("1"), 16).unwrap()), "2.7182818284590452");
        assert_eq!(
            format(&exp(&n("50"), 25).unwrap()),
            "5184705528587072464087.4533229334853848274691006"
        );
        assert_eq!(format(&ln(&n("1"), 16).unwrap()), "0");
        assert_eq!(format(&ln(&exp(&n("2"), 40).unwrap(), 16).unwrap()), "2");
        assert_eq!(format(&ln(&n("10"), 16).unwrap()), "2.3025850929940457");
        assert_eq!(format(&pow(&n("2"), &n("10"), 16).unwrap()), "1024");
        assert_eq!(format(&pow(&n("2"), &n("-1"), 16).unwrap()), "0.5");
        assert_eq!(format(&pow(&n("4"), &n("0.5"), 16).unwrap()), "2");
        assert!(pow(&n("-4"), &n("0.5"), 16).is_none());
    }
}
