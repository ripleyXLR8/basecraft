//! Calculations stored in a file: decompiles the token stream (key 5 of a
//! calculation node) back to FileMaker source text.
//!
//! A calculation node holds key 4 (context table occurrence), key 5 (token
//! stream that keeps the source formatting) and key 6 (compiled form).

use crate::calc_tables::{FUNCTIONS, FUNCTIONS2, GET_FLAGS, OPS};
use crate::graph::to_ref;
use crate::{path_int, text};

/// Resolves names referenced by a calculation.
pub trait Names {
    /// Field name of `field` in the base table behind table occurrence `to`.
    fn field(&self, to: u32, field: u32) -> Option<String>;
    /// Name of table occurrence `to`.
    fn occurrence(&self, to: u32) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcError {
    pub at: usize,
    pub token: u8,
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown calculation token 0x{:02X} at {}",
            self.token, self.at
        )
    }
}

impl std::error::Error for CalcError {}

/// A text run in a token stream: `len` counts UTF-16 units, not bytes, and
/// the run is followed by a 0x00 byte. Returns the text and the byte length.
pub(crate) fn run(b: &[u8], start: usize, len: usize) -> Option<(String, usize)> {
    (len..=len * 3 + 3).find_map(|n| {
        let bytes = b.get(start..start + n)?;
        let s = text(bytes);
        (s.encode_utf16().count() == len && b.get(start + n) == Some(&0)).then_some((s, n))
    })
}

fn lookup<T: Copy + PartialEq>(table: &[(T, &'static str)], k: T) -> Option<&'static str> {
    table.iter().find(|(t, _)| *t == k).map(|(_, n)| *n)
}

/// Formats the 16-byte fixed-point value of a number literal:
/// u32 integer part, then two u32 groups of nine decimals.
fn number(v: &[u8]) -> String {
    let u = |i: usize| u32::from_be_bytes([v[i], v[i + 1], v[i + 2], v[i + 3]]) as u64;
    let int = u(0);
    let frac = format!("{:09}{:09}", u(4), u(8));
    let frac = frac.trim_end_matches('0');
    // ponytail: bytes 12–15 (flags, sign?) not decoded; only 0x0020 observed.
    if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    }
}

/// Decompiles a token stream. `context` is the calculation's table occurrence.
pub fn decompile(b: &[u8], context: u32, names: &dyn Names) -> Result<String, CalcError> {
    let mut out = String::new();
    let mut i = 0;
    let err = |at: usize| CalcError {
        at,
        token: b.get(at).copied().unwrap_or(0),
    };
    let get = |i: usize| b.get(i).copied();
    while i < b.len() {
        let t = b[i];
        match t {
            // Text literal: 13 len text 00
            0x13 => {
                let len = get(i + 1).ok_or_else(|| err(i))? as usize;
                let (s, n) = run(b, i + 2, len).ok_or_else(|| err(i))?;
                out.push('"');
                out.push_str(&s.replace('\\', "\\\\").replace('"', "\\\""));
                out.push('"');
                i += 3 + n;
            }
            // Whitespace or comment: 0C followed by a text literal, emitted verbatim.
            0x0C => {
                if get(i + 1) != Some(0x13) {
                    return Err(err(i));
                }
                let len = get(i + 2).ok_or_else(|| err(i))? as usize;
                let (s, n) = run(b, i + 3, len).ok_or_else(|| err(i))?;
                out.push_str(&s);
                i += 4 + n;
            }
            // Let variable name: 0B len name 00 + 1 byte.
            0x0B => {
                let len = get(i + 1).ok_or_else(|| err(i))? as usize;
                let (s, n) = run(b, i + 2, len).ok_or_else(|| err(i))?;
                out.push_str(&s);
                i += 4 + n;
            }
            // Variable ($local, $$global): 1A len name, no terminator.
            0x1A => {
                let len = get(i + 1).ok_or_else(|| err(i))? as usize;
                let (s, n) = (len..=len * 3 + 3)
                    .find_map(|n| {
                        let s = text(b.get(i + 2..i + 2 + n)?);
                        (s.encode_utf16().count() == len).then_some((s, n))
                    })
                    .ok_or_else(|| err(i))?;
                out.push_str(&s);
                i += 2 + n;
            }
            // Number literal: 10 + 5 header bytes + 14-byte value block.
            0x10 => {
                let v = b.get(i + 6..i + 20).ok_or_else(|| err(i))?;
                let mut full = v.to_vec();
                full.resize(16, 0);
                out.push_str(&number(&full));
                i += 20;
            }
            // Field reference: 16 flags | 04 TOref | len (01 field) | u16 repetition
            0x16 => {
                let to = b.get(i + 3..i + 7).and_then(to_ref).ok_or_else(|| err(i))?;
                let n = get(i + 7).ok_or_else(|| err(i))? as usize;
                let field = b
                    .get(i + 9..i + 8 + n)
                    .and_then(path_int)
                    .ok_or_else(|| err(i))?;
                let fname = names
                    .field(to, field)
                    .unwrap_or_else(|| "<Field Missing>".into());
                if to != context {
                    let tname = names
                        .occurrence(to)
                        .unwrap_or_else(|| "<Table Missing>".into());
                    out.push_str(&tname);
                    out.push_str("::");
                }
                out.push_str(&fname);
                i += 8 + n + 2;
            }
            // Get ( flag )
            0x9C => {
                let f = get(i + 1).ok_or_else(|| err(i))?;
                out.push_str(lookup(GET_FLAGS, f).ok_or_else(|| err(i))?);
                i += 2;
            }
            0xF8..=0xFE => {
                let v = get(i + 1).ok_or_else(|| err(i))?;
                let n = FUNCTIONS2
                    .iter()
                    .find(|(p, x, _)| *p == t && *x == v)
                    .map(|(_, _, n)| *n)
                    .ok_or_else(|| err(i))?;
                out.push_str(n);
                i += 2;
            }
            _ => {
                let s = lookup(OPS, t)
                    .or_else(|| lookup(FUNCTIONS, t))
                    .ok_or_else(|| err(i))?;
                out.push_str(s);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Context table occurrence of a calculation node (key 4), when it is a TO.
pub fn context(key4: &[u8]) -> Option<u32> {
    to_ref(key4)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct N;
    impl Names for N {
        fn field(&self, _: u32, f: u32) -> Option<String> {
            Some(format!("F{f}"))
        }
        fn occurrence(&self, t: u32) -> Option<String> {
            Some(format!("T{t}"))
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn get_uuid_with_spacing() {
        let b = hex("9b0c13017a00040c13017a009c650c13017a0005");
        assert_eq!(decompile(&b, 130, &N).unwrap(), "Get ( UUID )");
    }

    #[test]
    fn concatenation_with_fields_and_literal() {
        // First Name & " " & Last Name, context TO 132
        let b = hex(
            "16000403d0000402010100010c13017a00500c13017a0013017a000c13017a00500c13017a0016000403d000040201020001",
        );
        assert_eq!(decompile(&b, 132, &N).unwrap(), "F1 & \" \" & F2");
    }

    #[test]
    fn variables() {
        // $x & "y", from a Set Variable step
        let b = hex("1a027e220c13017a00500c13017a0013012300");
        assert_eq!(decompile(&b, 0, &N).unwrap(), "$x & \"y\"");
    }

    #[test]
    fn number_literals() {
        let one = hex("1002000100100000000100000000000000000020");
        assert_eq!(decompile(&one, 130, &N).unwrap(), "1");
        let mut frac = one.clone();
        frac[10] = 0x05; // 0x05000000 / 1e9
        assert_eq!(decompile(&frac, 130, &N).unwrap(), "1.08388608");
    }
}
