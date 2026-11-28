//! Calculations stored in a file: decompiles the token stream (key 5 of a
//! calculation node) back to FileMaker source text, and compiles source
//! text into both stored forms ([`compile`]).
//!
//! A calculation node holds key 4 (context table occurrence), key 5 (token
//! stream that keeps the source formatting) and key 6 (compiled form).

pub use crate::calc_compile::{
    CompileError, Compiled, Resolve, compile, compile_function, compiled, function_names,
};
use crate::calc_tables::{EXTERNAL, FUNCTIONS, FUNCTIONS2, GET_FLAGS, OPS};
use crate::graph::to_ref;
use crate::{path_int, text};

/// Resolves names referenced by a calculation.
pub trait Names {
    /// Field name of `field` in the base table behind table occurrence `to`.
    fn field(&self, to: u32, field: u32) -> Option<String>;
    /// Name of table occurrence `to`.
    fn occurrence(&self, to: u32) -> Option<String>;
    /// Name of custom function `id` (File › Manage › Custom Functions).
    fn custom_function(&self, id: u32) -> Option<String> {
        let _ = id;
        None
    }
}

/// Custom function reference after a `1B` token: `14 len 01 id` (the
/// id spelled as a path element). Returns (id, bytes).
pub(crate) fn custom_ref(b: &[u8]) -> Option<(u32, usize)> {
    if *b.first()? != 0x14 {
        return None;
    }
    let n = *b.get(1)? as usize;
    if n < 2 || *b.get(2)? != 1 {
        return None;
    }
    Some((path_int(b.get(3..2 + n)?)?, 2 + n))
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

/// Length of a text run at `b[i]`: one byte, or two with the top bit set
/// (from 128 UTF-16 units). Returns (length, bytes used).
pub(crate) fn run_len(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let x = *b.get(i)? as usize;
    if x < 0x80 {
        Some((x, 1))
    } else {
        Some((((x & 0x7F) << 8) | *b.get(i + 1)? as usize, 2))
    }
}

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

/// Length of a number literal token at the start of `b`, in its compiled
/// form (key 5 adds two flag bytes): `10 02`, u16 n, u16 l, n u32 groups of
/// nine integer digits, then ⌈(l+2)/9⌉ u32 groups of nine decimals (l ≥ 16
/// decimals, plus two guard digits).
pub(crate) fn number_len(b: &[u8]) -> Option<usize> {
    let u = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize);
    let (n, l) = (u(2)?, u(4)?);
    Some(6 + 4 * n + 4 * (l + 2).div_ceil(9))
}

/// Value of the number literal token at the start of `b`, as plain decimal text.
fn number(b: &[u8]) -> Option<String> {
    let len = number_len(b)?;
    let v = b.get(..len)?;
    let n = u16::from_be_bytes([v[2], v[3]]) as usize;
    let groups: Vec<String> = v[6..]
        .chunks(4)
        .map(|w| format!("{:09}", u32::from_be_bytes([w[0], w[1], w[2], w[3]])))
        .collect();
    let int = groups[..n].concat();
    let int = int.trim_start_matches('0');
    let frac = groups[n..].concat();
    let frac = frac.trim_end_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    Some(if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    })
}

/// `1000` → `1E3`, `0.01` → `1E-2`, `150` → `1.5E2`.
fn scientific(plain: &str) -> String {
    let (int, frac) = plain.split_once('.').unwrap_or((plain, ""));
    let digits: String = format!("{int}{frac}");
    let lead = digits.len() - digits.trim_start_matches('0').len();
    let sig = digits.trim_matches('0');
    if sig.is_empty() {
        return "0".into();
    }
    let exp = int.len() as i64 - lead as i64 - 1;
    let (a, b) = sig.split_at(1);
    if b.is_empty() {
        format!("{a}E{exp}")
    } else {
        format!("{a}.{b}E{exp}")
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
                let (len, l) = run_len(b, i + 1).ok_or_else(|| err(i))?;
                let (s, n) = run(b, i + 1 + l, len).ok_or_else(|| err(i))?;
                out.push('"');
                // A pilcrow character is written `\¶` (a bare ¶ means a return).
                let s = s.replace('\\', "\\\\").replace('"', "\\\"");
                out.push_str(&s.replace('¶', "\\¶"));
                out.push('"');
                i += 2 + l + n;
            }
            // Whitespace (0C), /* comment */ (0D), // comment (0E, with its
            // line break): a text run, emitted verbatim.
            0x0C..=0x0E => {
                if get(i + 1) != Some(0x13) {
                    return Err(err(i));
                }
                let (len, l) = run_len(b, i + 2).ok_or_else(|| err(i))?;
                let (s, n) = run(b, i + 2 + l, len).ok_or_else(|| err(i))?;
                match t {
                    0x0D => {
                        out.push_str("/*");
                        out.push_str(&s);
                        out.push_str("*/");
                    }
                    0x0E => {
                        out.push_str("//");
                        out.push_str(&s);
                    }
                    _ => out.push_str(&s),
                }
                i += 3 + l + n;
            }
            // Let variable name: 0B len name 00 + 1 byte.
            0x0B => {
                let (len, l) = run_len(b, i + 1).ok_or_else(|| err(i))?;
                let (s, n) = run(b, i + 1 + l, len).ok_or_else(|| err(i))?;
                out.push_str(&s);
                i += 3 + l + n;
            }
            0x24 => {
                out.push('¶');
                i += 1;
            }
            // Library function: 1D 00 00 15, library name, u16 index.
            0x1D => {
                let lib = b.get(i + 4..i + 8).ok_or_else(|| err(i))?;
                let idx = b
                    .get(i + 8..i + 10)
                    .map(|x| u16::from_be_bytes([x[0], x[1]]))
                    .ok_or_else(|| err(i))?;
                let name = EXTERNAL
                    .iter()
                    .find(|(l, x, _)| l.as_slice() == lib && *x == idx)
                    .map(|(_, _, n)| *n)
                    .ok_or_else(|| err(i))?;
                out.push_str(name);
                i += 10;
            }
            // Custom function: 1B 14 len 01 id (FORMAT.md §16).
            0x1B => {
                let (id, n) = custom_ref(&b[i + 1..]).ok_or_else(|| err(i))?;
                let name = names
                    .custom_function(id)
                    .unwrap_or_else(|| "<Function Missing>".into());
                out.push_str(&name);
                i += 1 + n;
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
            // Number literal: 10 02 n l, groups of nine digits, two flag bytes.
            0x10 => {
                let len = number_len(&b[i..]).ok_or_else(|| err(i))?;
                let n = number(&b[i..]).ok_or_else(|| err(i))?;
                // Flags 0x40: the literal was written with an exponent.
                out.push_str(&if b.get(i + len + 1) == Some(&0x40) {
                    scientific(&n)
                } else {
                    n
                });
                i += len + 2;
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
                // Flag 04: written `TO::Field`.
                if to != context || get(i + 1).is_some_and(|f| f & 4 != 0) {
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

/// The names of a file's schema, for [`decompile`] and [`compile`].
#[derive(Debug, Clone, Default)]
pub struct FileNames {
    pub tables: Vec<crate::schema::Table>,
    pub occurrences: Vec<crate::graph::TableOccurrence>,
    /// Custom functions: (id, name).
    pub custom_functions: Vec<(u32, String)>,
}

impl FileNames {
    pub fn load(file: &crate::File) -> Result<Self, crate::Error> {
        Ok(FileNames {
            tables: crate::schema::tables(file)?,
            occurrences: crate::graph::table_occurrences(file)?,
            custom_functions: crate::custom_functions::read(file)?
                .into_iter()
                .filter(|f| f.kind == crate::custom_functions::Kind::Function)
                .map(|f| (f.id, f.name))
                .collect(),
        })
    }

    fn table(&self, to: u32) -> Option<&crate::schema::Table> {
        let bt = self.occurrences.iter().find(|t| t.id == to)?.base_table;
        self.tables.iter().find(|t| t.id == bt)
    }
}

impl Names for FileNames {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        Some(self.table(to)?.fields.get(&field)?.name.clone())
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(self.occurrences.iter().find(|t| t.id == to)?.name.clone())
    }
    fn custom_function(&self, id: u32) -> Option<String> {
        let (_, n) = self.custom_functions.iter().find(|(i, _)| *i == id)?;
        Some(n.clone())
    }
}

impl Resolve for FileNames {
    fn occurrences(&self) -> Vec<(u32, String)> {
        self.occurrences
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect()
    }
    fn fields(&self, to: u32) -> Vec<(u32, String)> {
        self.table(to).map_or_else(Vec::new, |t| {
            t.fields.iter().map(|(i, f)| (*i, f.name.clone())).collect()
        })
    }
    fn custom_functions(&self) -> Vec<(u32, String)> {
        self.custom_functions.clone()
    }
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
        // Pasted into FileMaker 26: integers of 1e9 and more, 25 decimals.
        let big = hex("1002000400100000007b1b3a0c14149aa4350dfb38d200000000000000000020");
        assert_eq!(
            decompile(&big, 130, &N).unwrap(),
            "123456789012345678901234567890"
        );
        let fine = hex("10020001001900000000075bcd1500bc614e35b7bf440020");
        assert_eq!(
            decompile(&fine, 130, &N).unwrap(),
            "0.1234567890123456789012345"
        );
        let exp = hex("100200040010000003e800000000000000000000000000000000000000000040");
        assert_eq!(decompile(&exp, 130, &N).unwrap(), "1E30");
    }
}
