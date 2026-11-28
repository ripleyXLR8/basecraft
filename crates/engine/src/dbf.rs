//! dBASE III DBF files, as FileMaker's "dBASE III and IV DBF" format
//! documents them: field names of ten characters at most, uppercase, spaces
//! as underscores, no duplicates; at most 128 fields, 254 characters per
//! field and 4000 bytes per record; the first repetition only.

use calc::Value;
use fmp12::schema::{DataType, Locale};

use crate::charset::{self, Charset};

/// The DBF name of a field: uppercase, spaces (and other characters a DBF
/// name cannot hold) as underscores, cut to ten characters.
pub fn field_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(10)
        .collect()
}

/// Language driver byte (header offset 29) of a character set.
fn driver(cs: Charset) -> u8 {
    match cs {
        Charset::WindowsAnsi => 0x03,
        Charset::MacRoman => 0x04,
        _ => 0x00,
    }
}

/// A DBF file of `columns` (name, type) and `rows` (stored texts in `loc`).
pub fn write(
    columns: &[(String, DataType)],
    rows: &[Vec<String>],
    loc: &Locale,
    cs: Charset,
) -> Result<Vec<u8>, String> {
    // UTF-16 has no place in a DBF file: Windows ANSI instead.
    let cs = if cs == Charset::Utf16 {
        Charset::WindowsAnsi
    } else {
        cs
    };
    if columns.len() > 128 {
        return Err("a DBF file holds at most 128 fields".into());
    }
    let names: Vec<String> = columns.iter().map(|c| field_name(&c.0)).collect();
    for (i, n) in names.iter().enumerate() {
        if names[..i].contains(n) {
            return Err(format!(
                "two fields have the DBF name {n:?} (ten characters, uppercase)"
            ));
        }
    }
    // Cells as (bytes), per column type and width.
    let cell = |ci: usize, s: &str| -> Vec<u8> {
        match columns[ci].1 {
            DataType::Date => match crate::typed_in(s, DataType::Date, loc) {
                Value::Date(d) => {
                    let (y, m, d) = calc::value::civil(d);
                    format!("{y:04}{m:02}{d:02}").into_bytes()
                }
                _ => Vec::new(),
            },
            DataType::Number => match crate::typed_in(s, DataType::Number, loc) {
                v @ Value::Number(_) => v.to_text().into_bytes(),
                _ => Vec::new(),
            },
            _ => {
                let mut b = charset::encode(s, cs);
                b.truncate(254);
                b
            }
        }
    };
    let cells: Vec<Vec<Vec<u8>>> = rows
        .iter()
        .map(|r| {
            (0..columns.len())
                .map(|ci| cell(ci, r.get(ci).map_or("", |s| s.as_str())))
                .collect()
        })
        .collect();
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|c| if c.1 == DataType::Date { 8 } else { 1 })
        .collect();
    let mut decimals = vec![0usize; columns.len()];
    for row in &cells {
        for (ci, b) in row.iter().enumerate() {
            if columns[ci].1 == DataType::Number
                && let Some(p) = b.iter().position(|&c| c == b'.')
            {
                decimals[ci] = decimals[ci].max(b.len() - p - 1).min(15);
            }
        }
    }
    // Numbers with the column's decimals (`200` as `200.0`).
    let mut cells = cells;
    for row in cells.iter_mut() {
        for (ci, b) in row.iter_mut().enumerate() {
            if columns[ci].1 == DataType::Number && !b.is_empty() && decimals[ci] > 0 {
                let have = b.iter().position(|&c| c == b'.').map(|p| b.len() - p - 1);
                if have.is_none() {
                    b.push(b'.');
                }
                b.resize(b.len() + decimals[ci] - have.unwrap_or(0), b'0');
            }
            if columns[ci].1 != DataType::Date {
                widths[ci] = widths[ci].max(b.len()).min(254);
            }
        }
    }
    let record_len = 1 + widths.iter().sum::<usize>();
    if record_len > 4000 {
        return Err("a DBF record holds at most 4000 bytes".into());
    }
    let header_len = 32 + 32 * columns.len() + 1;
    let (y, m, d) = today();
    let mut out = vec![0x03, (y - 1900).clamp(0, 255) as u8, m, d];
    out.extend((rows.len() as u32).to_le_bytes());
    out.extend((header_len as u16).to_le_bytes());
    out.extend((record_len as u16).to_le_bytes());
    out.extend([0u8; 17]);
    out.push(driver(cs));
    out.extend([0u8; 2]);
    for (ci, (_, dt)) in columns.iter().enumerate() {
        let mut name = [0u8; 11];
        for (i, b) in names[ci].bytes().enumerate() {
            name[i] = b;
        }
        out.extend(name);
        out.push(match dt {
            DataType::Number => b'N',
            DataType::Date => b'D',
            _ => b'C',
        });
        out.extend([0u8; 4]);
        out.push(widths[ci] as u8);
        out.push(decimals[ci] as u8);
        out.extend([0u8; 14]);
    }
    out.push(0x0D);
    for row in cells {
        out.push(b' ');
        for (ci, mut b) in row.into_iter().enumerate() {
            b.truncate(widths[ci]);
            let pad = vec![b' '; widths[ci] - b.len()];
            if columns[ci].1 == DataType::Number {
                out.extend(pad);
                out.extend(b);
            } else {
                out.extend(b);
                out.extend(pad);
            }
        }
    }
    out.push(0x1A);
    Ok(out)
}

/// Today as (year, month, day); 1900-01-01 where the clock cannot be read.
fn today() -> (i64, u8, u8) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        let day = calc::value::day_number(1970, 1, 1) + (d.as_secs() / 86_400) as i64;
        let (y, m, d) = calc::value::civil(day);
        return (y, m as u8, d as u8);
    }
    (1900, 1, 1)
}

/// Rows of a DBF file: the field names, then one row per record not marked
/// deleted. Dates (`D`) stay `YYYYMMDD` (the import converts them); memo
/// fields (`M`) are empty.
// ponytail: memo text lives in the `.dbt` file next to the `.dbf`; not read.
pub fn read(bytes: &[u8], cs: Charset) -> Result<Vec<Vec<String>>, String> {
    let bad = || "not a DBF file".to_string();
    if bytes.len() < 33 {
        return Err(bad());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let (header_len, record_len) = (u16_at(8), u16_at(10));
    let cs = match bytes[29] {
        0x03 => Charset::WindowsAnsi,
        0x04 => Charset::MacRoman,
        _ => cs,
    };
    let mut fields = Vec::new();
    let mut i = 32;
    while i + 32 <= bytes.len() && bytes[i] != 0x0D && i + 32 <= header_len {
        let d = &bytes[i..i + 32];
        let end = d[..11].iter().position(|&b| b == 0).unwrap_or(11);
        let name = charset::decode(&d[..end], cs);
        fields.push((name, d[11], d[16] as usize));
        i += 32;
    }
    if fields.is_empty() || record_len == 0 {
        return Err(bad());
    }
    let mut rows = vec![fields.iter().map(|f| f.0.clone()).collect::<Vec<_>>()];
    for r in 0..count {
        let start = header_len + r * record_len;
        let Some(rec) = bytes.get(start..start + record_len) else {
            break;
        };
        if rec[0] == b'*' {
            continue;
        }
        let mut at = 1;
        let mut row = Vec::new();
        for (_, ty, len) in &fields {
            let raw = rec.get(at..at + len).unwrap_or(&[]);
            at += len;
            row.push(match ty {
                b'M' => String::new(),
                _ => charset::decode(raw, cs).trim().to_string(),
            });
        }
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads() {
        let cols = vec![
            ("First name".to_string(), DataType::Text),
            ("Amount".to_string(), DataType::Number),
            ("Born".to_string(), DataType::Date),
        ];
        let rows = vec![
            vec!["Zoë".to_string(), "10.5".into(), "12/10/1815".into()],
            vec!["Bob".to_string(), "200".into(), String::new()],
        ];
        let loc = Locale::default();
        let b = write(&cols, &rows, &loc, Charset::WindowsAnsi).unwrap();
        assert_eq!(b[0], 0x03);
        assert_eq!(u32::from_le_bytes([b[4], b[5], b[6], b[7]]), 2);
        assert_eq!(&b[32..42], b"FIRST_NAME");
        // C 3, N 5 with 1 decimal, D 8.
        assert_eq!(
            (b[32 + 16], b[64 + 11], b[64 + 16], b[64 + 17]),
            (3, b'N', 5, 1)
        );
        let body = &b[32 + 96 + 1..];
        assert_eq!(&body[..17], b" Zo\xEB 10.518151210");
        assert_eq!(*b.last().unwrap(), 0x1A);
        let back = read(&b, Charset::Utf8).unwrap();
        assert_eq!(back[0], ["FIRST_NAME", "AMOUNT", "BORN"]);
        assert_eq!(back[1], ["Zoë", "10.5", "18151210"]);
        assert_eq!(back[2], ["Bob", "200.0", ""]);
        // Names that collide once cut to ten characters.
        let dup = vec![
            ("Customer name".to_string(), DataType::Text),
            ("Customer names".to_string(), DataType::Text),
        ];
        assert!(write(&dup, &[], &loc, Charset::Utf8).is_err());
    }
}
