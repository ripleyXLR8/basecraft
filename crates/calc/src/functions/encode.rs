//! Character codes, URL/Base64/hex encodings, digests and text encodings.

use super::{int, txt};
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R};
use crate::num;
use crate::value::{Container, Value};
use bigdecimal::Signed;
use bigdecimal::num_bigint::BigInt;
use sha2::Digest;

/// Bytes of a value: container data, or the UTF-8 text.
pub(crate) fn bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Container(c) => c.data.clone(),
        v => v.to_text().into_bytes(),
    }
}

/// Char ( 9800097 ) = "ab": five decimal digits per UTF-16 code unit,
/// first unit in the lowest digits (an emoji takes two).
// ponytail: a lone surrogate becomes U+FFFD; FileMaker keeps it.
pub fn char(_: &mut Evaluator, a: &[Value]) -> R {
    let n = num::trunc(&num::shown(&a[0].num_or_zero()), 0);
    if n.is_negative() {
        return Ok(Value::empty());
    }
    let digits = n.with_scale(0).into_bigint_and_exponent().0.to_string();
    let mut units = Vec::new();
    let mut end = digits.len();
    while end > 0 {
        let start = end.saturating_sub(5);
        let code: u32 = digits[start..end].parse().unwrap_or(0);
        if code != 0 && code <= 0xFFFF {
            units.push(code as u16);
        }
        end = start;
    }
    Ok(Value::Text(String::from_utf16_lossy(&units)))
}

/// UTF-16 code units, five decimal digits each, first in the lowest digits:
/// Code ( "😀" ) = 5683255357.
pub fn code(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    if s.is_empty() {
        return Ok(Value::empty());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    let total = units
        .iter()
        .rev()
        .fold(BigInt::from(0u8), |acc, &c| acc * 100_000u32 + c as u32);
    Ok(Value::num(num::Num::from(total)))
}

/// Percent-encodes UTF-8 bytes except letters, digits and `-_.!~*'():`.
pub fn url_encode(_: &mut Evaluator, a: &[Value]) -> R {
    let mut out = String::new();
    for b in txt(a, 0).bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'():".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(Value::Text(out))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn b64(data: &[u8]) -> String {
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

/// Lines of `width` characters, each ended by CRLF (FileMaker ends the last
/// line too).
fn wrap(s: &str, width: usize) -> String {
    s.as_bytes()
        .chunks(width)
        .map(|c| format!("{}\r\n", String::from_utf8_lossy(c)))
        .collect()
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
            format!("{}={}\r\n", wrap(&s, 76), b64(&crc[1..]))
        }
        _ => s, // 3548, 4648 and unknown values: no line breaks
    }))
}

/// A container when a file name is given, else UTF-8 text.
fn decoded(data: Vec<u8>, name: Option<String>) -> Value {
    match name.filter(|n| !n.is_empty()) {
        Some(name) => Value::Container(Container {
            name,
            data,
            ..Default::default()
        }),
        None => Value::Text(String::from_utf8_lossy(&data).into_owned()),
    }
}

fn bad_encoding(what: &str) -> EvalError {
    EvalError::new(code::BAD_ENCODING, format!("{what}: malformed input"))
}

/// Decoding stops at the first `=`; other characters outside the alphabet
/// are skipped. Without padding the characters must come in fours.
pub fn base64_decode(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let (body, padded) = match s.split_once('=') {
        Some((b, _)) => (b, true),
        None => (s.as_str(), false),
    };
    let n = body.bytes().filter(|c| B64.contains(c)).count();
    if !padded && n % 4 != 0 {
        return Err(bad_encoding("Base64Decode"));
    }
    let body: String = body.chars().filter(|c| *c != '-' && *c != '_').collect();
    Ok(decoded(b64_decode(&body), a.get(1).map(Value::to_text)))
}

/// Standard or URL-safe Base64; other characters (line breaks, padding)
/// are skipped.
pub(crate) fn b64_decode(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
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
    out
}

pub fn hex_encode(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(
        bytes(&a[0]).iter().map(|b| format!("{b:02X}")).collect(),
    ))
}

/// White space is skipped; any other non-hex character or an odd number of
/// digits is an error.
pub fn hex_decode(_: &mut Evaluator, a: &[Value]) -> R {
    let s = txt(a, 0);
    let digits = s.chars().filter(|c| !c.is_whitespace());
    if digits.clone().any(|c| !c.is_ascii_hexdigit()) || digits.count() % 2 != 0 {
        return Err(bad_encoding("HexDecode"));
    }
    Ok(decoded(hex_bytes(&s), a.get(1).map(Value::to_text)))
}

pub(crate) fn hex_bytes(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s
        .chars()
        .filter_map(|c| c.to_digit(16))
        .map(|d| d as u8)
        .collect();
    digits
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| p[0] << 4 | p[1])
        .collect()
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

/// Digests are containers named "digest.data".
fn bin(data: Vec<u8>) -> R {
    Ok(Value::Container(Container {
        name: "digest.data".into(),
        data,
        ..Default::default()
    }))
}

fn bad_alg(alg: &str) -> EvalError {
    EvalError::new(code::BAD_OPTION, format!("unsupported algorithm {alg}"))
}

pub fn crypt_digest(_: &mut Evaluator, a: &[Value]) -> R {
    let alg = txt(a, 1);
    bin(digest(&alg, &bytes(&a[0])).ok_or_else(|| bad_alg(&alg))?.0)
}

/// HMAC (RFC 2104). With an empty key FileMaker returns the data as is.
pub fn crypt_auth_code(_: &mut Evaluator, a: &[Value]) -> R {
    let alg = txt(a, 1);
    let (data, mut key) = (bytes(&a[0]), bytes(&a[2]));
    let (_, block) = digest(&alg, b"").ok_or_else(|| bad_alg(&alg))?;
    if key.is_empty() {
        return bin(data);
    }
    if key.len() > block {
        key = digest(&alg, &key).ok_or_else(|| bad_alg(&alg))?.0;
    }
    key.resize(block, 0);
    let pad = |p: u8| key.iter().map(|k| k ^ p).collect::<Vec<u8>>();
    let inner = digest(&alg, &[pad(0x36), data].concat())
        .ok_or_else(|| bad_alg(&alg))?
        .0;
    bin(digest(&alg, &[pad(0x5C), inner].concat())
        .ok_or_else(|| bad_alg(&alg))?
        .0)
}

#[derive(Clone, Copy, PartialEq)]
enum Codec {
    Utf8,
    Latin1,
    Cp1252,
}

/// windows-1252 bytes 0x80..0x9F.
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

// ponytail: UTF-8, ISO-8859-1 and windows-1252 only; Shift_JIS, UTF-16 &
// co. need codec tables.
fn codec(enc: &str) -> Result<Codec, EvalError> {
    match enc.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "utf8" => Ok(Codec::Utf8),
        "iso88591" | "latin1" => Ok(Codec::Latin1),
        "windows1252" | "cp1252" => Ok(Codec::Cp1252),
        _ => Err(EvalError::new(
            code::BAD_OPTION,
            format!("unsupported encoding {enc}"),
        )),
    }
}

/// TextEncode: line endings 2 = CR, 3 = LF, 4 = CR LF, others unchanged
/// (CR, FileMaker's own). The container is named after the encoding.
pub fn text_encode(ev: &mut Evaluator, a: &[Value]) -> R {
    let enc = txt(a, 1);
    // An encoding FileMaker does not know ("utf-16" included) gives "?",
    // with error 4 for EvaluationError.
    let codec = match codec(&enc) {
        Ok(c) => c,
        Err(e) => {
            ev.sticky.get_or_insert(e);
            return Ok(Value::text("?"));
        }
    };
    let s = txt(a, 0);
    let eol = match int(a, 2)? {
        2 => Some("\r"),
        3 => Some("\n"),
        4 => Some("\r\n"),
        _ => None,
    };
    let s = match eol {
        Some(eol) => s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', eol),
        None => s,
    };
    let data = match codec {
        Codec::Utf8 => s.into_bytes(),
        Codec::Latin1 | Codec::Cp1252 => s
            .chars()
            .map(|c| match c as u32 {
                u @ 0..=0x7F | u @ 0xA0..=0xFF => u as u8,
                u @ 0x80..=0x9F if codec == Codec::Latin1 => u as u8,
                _ => CP1252
                    .iter()
                    .position(|&x| x == c)
                    .map_or(b'?', |i| 0x80 + i as u8),
            })
            .collect(),
    };
    Ok(Value::Container(Container {
        name: format!("{enc}.txt"),
        data,
        ..Default::default()
    }))
}

/// TextDecode: line endings become CR.
pub fn text_decode(ev: &mut Evaluator, a: &[Value]) -> R {
    let codec = match codec(&txt(a, 1)) {
        Ok(c) => c,
        Err(e) => {
            ev.sticky.get_or_insert(e);
            return Ok(Value::text("?"));
        }
    };
    let data = bytes(&a[0]);
    let s: String = match codec {
        Codec::Utf8 => String::from_utf8_lossy(&data).into_owned(),
        Codec::Latin1 => data.iter().map(|&b| b as char).collect(),
        Codec::Cp1252 => data
            .iter()
            .map(|&b| match b {
                0x80..=0x9F => CP1252[b as usize - 0x80],
                b => b as char,
            })
            .collect(),
    };
    Ok(Value::Text(s.replace("\r\n", "\r").replace('\n', "\r")))
}
