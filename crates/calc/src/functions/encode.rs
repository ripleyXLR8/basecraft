//! Character codes, URL/Base64/hex encodings, digests and text encodings.

use super::{int, txt};
use crate::error::EvalError;
use crate::eval::{Evaluator, R};
use crate::num;
use crate::value::{Container, Value};
use bigdecimal::num_bigint::BigInt;
use sha2::Digest;

/// Bytes of a value: container data, or the UTF-8 text.
fn bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Container(c) => c.data.clone(),
        v => v.to_text().into_bytes(),
    }
}

/// Char ( 9800097 ) = "ab": five decimal digits per character, first
/// character in the lowest digits.
pub fn char(_: &mut Evaluator, a: &[Value]) -> R {
    let n = num::trunc(&a[0].num_or_zero(), 0);
    let digits = n.into_bigint_and_exponent().0.magnitude().to_string();
    let mut out = String::new();
    let mut end = digits.len();
    while end > 0 {
        let start = end.saturating_sub(5);
        let code: u32 = digits[start..end].parse().unwrap_or(0);
        if let Some(c) = char::from_u32(code).filter(|_| code != 0) {
            out.push(c);
        }
        end = start;
    }
    Ok(Value::Text(out))
}

pub fn code(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    if s.is_empty() {
        return Ok(Value::empty());
    }
    let total = s
        .chars()
        .rev()
        .fold(BigInt::from(0u8), |acc, c| acc * 100_000u32 + c as u32);
    Ok(Value::num(num::Num::from(total)))
}

pub fn url_encode(_: &mut Evaluator, a: &[Value]) -> R {
    let mut out = String::new();
    for b in txt(a, 0).bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(Value::Text(out))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn wrap(s: &str, width: usize) -> String {
    let lines: Vec<&str> = s
        .as_bytes()
        .chunks(width)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    lines.join("\r\n")
}

/// OpenPGP CRC-24 (RFC 4880).
fn crc24(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xB704CE;
    for &b in data {
        crc ^= (b as u32) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x1000000 != 0 {
                crc ^= 0x1864CFB;
            }
        }
    }
    crc & 0xFFFFFF
}

/// RFC 2045 formatting: 76-character lines separated by CRLF.
pub fn base64_encode(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(wrap(&b64(&bytes(&a[0])), 76)))
}

pub fn base64_encode_rfc(_: &mut Evaluator, a: &[Value]) -> R {
    let data = bytes(&a[1]);
    let s = b64(&data);
    Ok(Value::Text(match int(a, 0)? {
        1421 => wrap(&s, 64),
        2045 => wrap(&s, 76),
        4880 => {
            let crc = crc24(&data).to_be_bytes();
            format!("{}\r\n={}", wrap(&s, 76), b64(&crc[1..]))
        }
        _ => s, // 3548, 4648 and unknown values: no line breaks
    }))
}

fn decoded(data: Vec<u8>, name: Option<String>) -> Value {
    match name {
        Some(name) => Value::Container(Container { name, data }),
        None => Value::Text(String::from_utf8_lossy(&data).into_owned()),
    }
}

pub fn base64_decode(_: &mut Evaluator, a: &[Value]) -> R {
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut out = Vec::new();
    for c in txt(a, 0).bytes() {
        let v = match c {
            b'-' => 62,
            b'_' => 63,
            c => match B64.iter().position(|&x| x == c) {
                Some(v) => v as u32,
                None => continue,
            },
        };
        bits = bits << 6 | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    Ok(decoded(out, a.get(1).map(Value::to_text)))
}

pub fn hex_encode(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(
        bytes(&a[0]).iter().map(|b| format!("{b:02X}")).collect(),
    ))
}

pub fn hex_decode(_: &mut Evaluator, a: &[Value]) -> R {
    let digits: Vec<u8> = txt(a, 0)
        .chars()
        .filter_map(|c| c.to_digit(16))
        .map(|d| d as u8)
        .collect();
    let out = digits
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| p[0] << 4 | p[1])
        .collect();
    Ok(decoded(out, a.get(1).map(Value::to_text)))
}

/// (digest, block size) for an algorithm name.
fn digest(alg: &str, data: &[u8]) -> Option<(Vec<u8>, usize)> {
    Some(match alg.to_ascii_uppercase().as_str() {
        "MD5" => (md5::Md5::digest(data).to_vec(), 64),
        "SHA1" => (sha1::Sha1::digest(data).to_vec(), 64),
        "SHA224" => (sha2::Sha224::digest(data).to_vec(), 64),
        "SHA256" => (sha2::Sha256::digest(data).to_vec(), 64),
        "SHA384" => (sha2::Sha384::digest(data).to_vec(), 128),
        "SHA512" => (sha2::Sha512::digest(data).to_vec(), 128),
        _ => return None,
    })
}

fn bin(data: Vec<u8>) -> R {
    Ok(Value::Container(Container {
        name: String::new(),
        data,
    }))
}

fn bad_alg(alg: &str) -> EvalError {
    EvalError::generic(format!("unsupported algorithm {alg}"))
}

pub fn crypt_digest(_: &mut Evaluator, a: &[Value]) -> R {
    let alg = txt(a, 1);
    bin(digest(&alg, &bytes(&a[0])).ok_or_else(|| bad_alg(&alg))?.0)
}

/// HMAC (RFC 2104).
pub fn crypt_auth_code(_: &mut Evaluator, a: &[Value]) -> R {
    let alg = txt(a, 1);
    let (data, mut key) = (bytes(&a[0]), bytes(&a[2]));
    let (_, block) = digest(&alg, b"").ok_or_else(|| bad_alg(&alg))?;
    if key.len() > block {
        key = digest(&alg, &key).unwrap().0;
    }
    key.resize(block, 0);
    let pad = |p: u8| key.iter().map(|k| k ^ p).collect::<Vec<u8>>();
    let inner = digest(&alg, &[pad(0x36), data].concat()).unwrap().0;
    bin(digest(&alg, &[pad(0x5C), inner].concat()).unwrap().0)
}

fn utf8(enc: &str) -> Result<(), EvalError> {
    // ponytail: UTF-8 only; other encodings need a codec table.
    match enc.to_ascii_lowercase().replace('-', "").as_str() {
        "utf8" => Ok(()),
        _ => Err(EvalError::generic(format!("unsupported encoding {enc}"))),
    }
}

pub fn text_encode(_: &mut Evaluator, a: &[Value]) -> R {
    utf8(&txt(a, 1))?;
    let s = txt(a, 0);
    let eol = match int(a, 2)? {
        2 => Some("\r"),
        3 => Some("\r\n"),
        4 => Some("\n"),
        _ => None,
    };
    let s = match eol {
        Some(eol) => s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', eol),
        None => s,
    };
    Ok(Value::Container(Container {
        name: "Untitled.txt".into(),
        data: s.into_bytes(),
    }))
}

pub fn text_decode(_: &mut Evaluator, a: &[Value]) -> R {
    utf8(&txt(a, 1))?;
    Ok(Value::Text(
        String::from_utf8_lossy(&bytes(&a[0])).into_owned(),
    ))
}
