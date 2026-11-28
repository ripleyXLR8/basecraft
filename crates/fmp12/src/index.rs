//! Field indexes `[T].[1].[F]` as FileMaker Pro 26 builds them (FORMAT.md
//! §13, "Field indexes"): the value index `[F].[0]` (one key per line of a
//! value, `01` for an empty one) and, for "All" indexing, the word index
//! `[F].[1]`. Keys were read from indexes FileMaker built for records a
//! script created. Values whose keys are not known (characters without
//! collation weights, unusual number or date spellings, times, long values)
//! give `None`: the caller then leaves the index for FileMaker to rebuild.

use std::collections::{BTreeMap, BTreeSet};

use unicode_segmentation::UnicodeSegmentation;

use crate::schema::{DataType, DateOrder, Field, FieldKind, Indexing, Locale, Records};

/// Index key → ids of the records holding it.
pub type Postings = BTreeMap<Vec<u8>, BTreeSet<u32>>;

#[derive(Debug, Default, PartialEq)]
pub struct FieldIndex {
    /// `[F].[0]`.
    pub values: Postings,
    /// `[F].[1]`, for "All" indexing.
    pub words: Option<Postings>,
}

/// `[F]` key 16 (value index) or 17 (word index): u64 number of entries
/// (record ids over all keys), u64 number of keys.
pub fn stats(p: &Postings) -> Vec<u8> {
    let n: usize = p.values().map(BTreeSet::len).sum();
    [(n as u64).to_be_bytes(), (p.len() as u64).to_be_bytes()].concat()
}

// ponytail: lines and words over 100 characters fall back to a rebuild;
// FileMaker cuts keys after 109 characters and adds `FF 80` (not decoded).
const MAX_CHARS: usize = 100;

fn text_key(s: &str) -> Option<Vec<u8>> {
    if s.chars().count() > MAX_CHARS {
        return None;
    }
    let mut k = vec![0x01];
    k.extend(crate::collate::primary_key(s)?);
    Some(k)
}

/// Number key: u16 `0x8000 + number of integer digits`, then the digits
/// (leading integer zeros dropped) as nibbles `digit + 1`, padded with 0;
/// a negative number is the complement of its absolute value's key with a
/// 0 nibble after the digits (`-2.5` = `7FFE C9FF`).
fn number_key(s: &str, decimal: char) -> Option<Vec<u8>> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let (int, frac) = body.split_once(decimal).unwrap_or((body, ""));
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    // ponytail: only plain spellings (no leading or trailing zeros beyond a
    // single `0`, no exponent, ≤ 30 digits); others fall back to a rebuild.
    if !digits(int)
        || !digits(frac)
        || (int.is_empty() && frac.is_empty())
        || (int.len() > 1 && int.starts_with('0'))
        || frac.ends_with('0')
        || int.len() + frac.len() > 30
    {
        return None;
    }
    let int = int.trim_start_matches('0');
    if int.is_empty() && frac.is_empty() {
        return (!neg).then(|| vec![0x80, 0x00]);
    }
    let mut out = (0x8000u16 + int.len() as u16).to_be_bytes().to_vec();
    let mut nib: Vec<u8> = int
        .bytes()
        .chain(frac.bytes())
        .map(|b| b - b'0' + 1)
        .collect();
    if neg {
        nib.push(0);
    }
    if nib.len() % 2 == 1 {
        nib.push(0);
    }
    out.extend(nib.chunks(2).map(|c| c[0] << 4 | c[1]));
    if neg {
        out.iter_mut().for_each(|b| *b = !*b);
    }
    Some(out)
}

/// Date key: `(0x2000 + year) << 10 | month << 6 | day` on 3 bytes, then
/// `80 00`. The value is read in the file's date order and separator, with a
/// four-digit year.
fn date_key(s: &str, loc: &Locale) -> Option<Vec<u8>> {
    let p: Vec<&str> = s.split(loc.date_separator).collect();
    let [a, b, c] = p.as_slice() else { return None };
    let (y, m, d) = match loc.date_order {
        DateOrder::Mdy => (c, a, b),
        DateOrder::Dmy => (c, b, a),
        DateOrder::Ymd => (a, b, c),
    };
    if y.len() != 4
        || [y, m, d]
            .iter()
            .any(|x| !x.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (y, m, d): (u32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=4000).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days[m as usize - 1] {
        return None;
    }
    let v = (0x2000 + y) << 10 | m << 6 | d;
    Some(vec![(v >> 16) as u8, (v >> 8) as u8, v as u8, 0x80, 0x00])
}

/// Value-index keys of one stored value (one per line).
fn value_keys(t: DataType, v: &str, loc: &Locale) -> Option<Vec<Vec<u8>>> {
    if v.is_empty() {
        return Some(vec![vec![0x01]]);
    }
    match t {
        DataType::Text => v
            .split('\r')
            .map(|line| {
                // ponytail: empty lines and lines with outer white space
                // fall back to a rebuild (not observed).
                if line.is_empty() || line.trim() != line {
                    return None;
                }
                text_key(line)
            })
            .collect(),
        DataType::Number => Some(vec![number_key(v, loc.decimal)?]),
        DataType::Date => Some(vec![date_key(v, loc)?]),
        _ => None,
    }
}

/// The index FileMaker keeps for `field` over `records` (value key
/// `id`), or `None` when the field has no index or a key is unknown.
pub fn build(id: u32, field: &Field, records: &Records, loc: &Locale) -> Option<FieldIndex> {
    let indexing = field.indexing();
    // ponytail: English and French index languages only (the keys were
    // checked in a French file; their primary weights are the same).
    if indexing == Indexing::None
        || field.kind() != FieldKind::Normal
        || field.global()
        || field.max_repetitions() > 1
        || !matches!(field.language(), 21 | 24)
    {
        return None;
    }
    let t = field.data_type();
    let all = indexing == Indexing::All;
    if all && t != DataType::Text {
        return None;
    }
    let mut out = FieldIndex {
        words: all.then(Postings::new),
        ..Default::default()
    };
    for (r, vals) in records {
        let v = vals.get(&id).map_or("", String::as_str);
        for k in value_keys(t, v, loc)? {
            out.values.entry(k).or_default().insert(*r);
        }
        if let Some(w) = out.words.as_mut() {
            for word in v.unicode_words() {
                w.entry(text_key(word)?).or_default().insert(*r);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Keys FileMaker Pro 26 built (French file: `,` decimal, DMY dates).
    #[test]
    fn keys_match_filemaker() {
        let loc = Locale {
            decimal: ',',
            date_order: DateOrder::Dmy,
            ..Default::default()
        };
        let n = |s| hex(&number_key(s, ',').unwrap());
        assert_eq!(n("3"), "800140");
        assert_eq!(n("-2,5"), "7ffec9ff");
        assert_eq!(n("10"), "800221");
        assert_eq!(n("0"), "8000");
        assert_eq!(n("1000000"), "800721111110");
        assert_eq!(n(",001"), "80001120");
        assert_eq!(n("-7"), "7ffe7f");
        assert_eq!(n("123456789012"), "800c23456789a123");
        assert_eq!(number_key("2,50", ','), None);
        let d = |s| hex(&date_key(s, &loc).unwrap());
        assert_eq!(d("02/01/2024"), "9fa0428000");
        assert_eq!(d("31/12/1999"), "9f3f1f8000");
        assert_eq!(d("01/01/0001"), "8004418000");
        assert_eq!(d("29/02/2000"), "9f409d8000");
        assert_eq!(date_key("29/02/2001", &loc), None);
        let t = |s| hex(&text_key(s).unwrap());
        assert_eq!(
            t("Café — crème"),
            "01123d120f12a3126b020a022c020a123d13da126b135f126b"
        );
        assert_eq!(t("ABC-12"), "01120f1225123d022312061207");
        assert_eq!(
            t("Straße Ölçü"),
            "011410143313da120f14101410126b020a138e1330123d1453"
        );
        assert_eq!(t("x1,yE+12y"), "01149712060234149c126b055012061207149c");
        let keys = value_keys(DataType::Text, "Line\rtwo", &loc).unwrap();
        assert_eq!(
            keys.iter().map(|k| hex(k)).collect::<Vec<_>>(),
            ["01133012ec136d126b", "011433148d138e"]
        );
        // Words as FileMaker split them.
        let w: Vec<&str> = "x1,yE+12y ABC-12 Café — crème".unicode_words().collect();
        assert_eq!(w, ["x1", "yE", "12y", "ABC", "12", "Café", "crème"]);
    }
}
