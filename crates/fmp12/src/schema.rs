//! Decoding of tables, fields and records from the path store.

use std::collections::BTreeMap;

use crate::{Chunk, Error, File, int_path, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Internal id (path element, >= 128).
    pub id: u32,
    pub name: String,
    pub fields: BTreeMap<u32, Field>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub comment: String,
    /// Raw 26-byte option block (key 2), kept for lossless writing.
    pub options: Vec<u8>,
    /// Raw summary definition of a summary field (key 9), see [`Field::summary`].
    pub summary_raw: Vec<u8>,
}

/// Operation of a summary field ("Total of", "Count of"...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryOp {
    Total,
    Average,
    Count,
    Minimum,
    Maximum,
    StandardDeviation,
    FractionOfTotal,
    List,
}

/// Definition of a summary field: the operation and the summarized field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub op: SummaryOp,
    /// Field id in the same table.
    pub field: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Normal,
    Calculated,
    Summary,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Text,
    Number,
    Date,
    Time,
    Timestamp,
    Container,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indexing {
    None,
    Minimal,
    All,
}

/// Auto-entered creation/modification value (option byte 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStamp {
    CreationDate,
    CreationTime,
    CreationTimestamp,
    CreationName,
    CreationAccountName,
    ModificationDate,
    ModificationTime,
    ModificationTimestamp,
    ModificationName,
    ModificationAccountName,
}

/// Validation flags (option bytes 14–15).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Validation {
    pub always: bool,
    pub allow_override: bool,
    pub not_empty: bool,
    pub unique: bool,
    pub existing: bool,
    pub range: bool,
    pub value_list: bool,
    pub max_size: bool,
    pub calculation: bool,
    pub custom_message: bool,
    pub strict_numeric: bool,
    pub strict_four_digit_year: bool,
    pub strict_time: bool,
}

impl Field {
    fn bit(&self, byte: usize, bit: u8) -> bool {
        self.options.get(byte).is_some_and(|b| b & (1 << bit) != 0)
    }
    fn byte(&self, i: usize) -> u8 {
        self.options.get(i).copied().unwrap_or(0)
    }

    pub fn kind(&self) -> FieldKind {
        match self.byte(0) {
            1 => FieldKind::Normal,
            2 => FieldKind::Calculated,
            3 => FieldKind::Summary,
            n => FieldKind::Unknown(n),
        }
    }

    pub fn data_type(&self) -> DataType {
        match self.byte(1) {
            1 => DataType::Text,
            2 => DataType::Number,
            3 => DataType::Date,
            4 => DataType::Time,
            5 => DataType::Timestamp,
            6 => DataType::Container,
            n => DataType::Unknown(n),
        }
    }

    /// Index language id (21 = English, Unicode ids above).
    pub fn language(&self) -> u32 {
        self.options
            .get(4..8)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn indexing(&self) -> Indexing {
        // ponytail: bit 3 also reads as Minimal; its exact role is unknown.
        if self.bit(8, 5) {
            Indexing::All
        } else if self.bit(8, 7) || self.bit(8, 3) {
            Indexing::Minimal
        } else {
            Indexing::None
        }
    }

    pub fn auto_index(&self) -> bool {
        !self.bit(8, 6)
    }
    pub fn global(&self) -> bool {
        self.bit(9, 0)
    }
    /// Calculation results are stored (unstored calcs clear this).
    pub fn stored(&self) -> bool {
        !self.bit(9, 1)
    }
    pub fn max_repetitions(&self) -> u16 {
        u16::from_be_bytes([self.byte(24), self.byte(25)])
    }

    pub fn prohibit_modification(&self) -> bool {
        self.bit(10, 0)
    }
    /// Serial number generated on commit (bit 10.1) or on creation (bit 11.1).
    pub fn auto_serial(&self) -> bool {
        self.bit(10, 1) || self.bit(11, 1)
    }
    pub fn auto_lookup(&self) -> bool {
        self.bit(10, 2)
    }
    pub fn auto_stamp(&self) -> Option<AutoStamp> {
        use AutoStamp::*;
        if !self.bit(11, 0) && self.byte(3) == 0 {
            return None;
        }
        [
            CreationDate,
            CreationTime,
            CreationTimestamp,
            CreationName,
            CreationAccountName,
            ModificationDate,
            ModificationTime,
            ModificationTimestamp,
            ModificationName,
            ModificationAccountName,
        ]
        .get(self.byte(3) as usize)
        .copied()
    }
    pub fn auto_constant(&self) -> bool {
        self.bit(11, 2)
    }
    pub fn auto_calculation(&self) -> bool {
        self.bit(11, 3)
    }
    pub fn auto_last_visited(&self) -> bool {
        self.bit(11, 4)
    }
    pub fn auto_always_evaluate(&self) -> bool {
        self.bit(11, 5)
    }
    pub fn auto_overwrite(&self) -> bool {
        self.bit(11, 7)
    }
    pub fn furigana(&self) -> bool {
        self.bit(11, 6)
    }

    /// Summary definition (key 9): byte 0 = operation, bytes 8.. = `02 len
    /// path-int field`. Operations 1 (Total) and 3 (Count) and the field
    /// position are seen in the templates (FORMAT.md §5).
    /// ponytail: the other operation numbers follow FileMaker's dialog order
    /// and are unverified; option bytes (running total, by population,
    /// restart per group, fraction subtotal field) are not decoded.
    pub fn summary(&self) -> Option<Summary> {
        use SummaryOp::*;
        if self.kind() != FieldKind::Summary {
            return None;
        }
        let b = &self.summary_raw;
        let op = match b.first()? {
            1 => Total,
            2 => Average,
            3 => Count,
            4 => Minimum,
            5 => Maximum,
            6 => StandardDeviation,
            7 => FractionOfTotal,
            8 => List,
            _ => return None,
        };
        // Bytes 8… are a field reference: length, 01, path integer.
        let n = (*b.get(8)? as usize).checked_sub(1)?;
        let field = crate::path_int(b.get(10..10 + n)?)?;
        Some(Summary { op, field })
    }

    /// Encodes a summary definition the way [`Field::summary`] reads it
    /// (databases built in code, tests).
    pub fn summary_bytes(s: Summary) -> Vec<u8> {
        use SummaryOp::*;
        let op = match s.op {
            Total => 1,
            Average => 2,
            Count => 3,
            Minimum => 4,
            Maximum => 5,
            StandardDeviation => 6,
            FractionOfTotal => 7,
            List => 8,
        };
        let mut b = vec![op, 0, 2, 1, 0, 0, 0xFE, 0x7F, 2, 1];
        b.push(s.field.min(127) as u8);
        b
    }

    pub fn validation(&self) -> Validation {
        Validation {
            value_list: self.bit(14, 0),
            max_size: self.bit(14, 1),
            always: self.bit(14, 2),
            strict_numeric: self.bit(14, 4),
            strict_four_digit_year: self.bit(14, 5),
            strict_time: self.bit(14, 6),
            custom_message: self.bit(14, 7),
            calculation: self.bit(15, 0),
            allow_override: !self.bit(15, 2),
            not_empty: self.bit(15, 3),
            unique: self.bit(15, 4),
            existing: self.bit(15, 5),
            range: self.bit(15, 6),
        }
    }
}

/// Record id → (field id → value).
pub type Records = BTreeMap<u32, BTreeMap<u32, String>>;

/// Accumulates a value stored either inline (key 16 of `node`) or as a long
/// value in sub-path `[node..., 16]` split over keys/segments 1, 2, ….
fn name_part(rel: &[u32], c: &Chunk) -> Option<(bool, String)> {
    match (rel, c) {
        ([], Chunk::Kv { key: 16, data }) => Some((true, text(data))),
        ([16], Chunk::Kv { key, data }) if *key >= 1 => Some((*key == 1, text(data))),
        _ => None,
    }
}

fn add(map: &mut BTreeMap<u32, String>, id: u32, (first, s): (bool, String)) {
    let e = map.entry(id).or_default();
    if first {
        e.clear();
    }
    e.push_str(&s);
}

pub fn tables(file: &File) -> Result<Vec<Table>, Error> {
    let mut names = BTreeMap::new();
    let mut fields: BTreeMap<u32, BTreeMap<u32, Field>> = BTreeMap::new();
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match p.as_slice() {
            [3, 16, 5, t, rel @ ..] if *t >= 128 => {
                if let Some(part) = name_part(rel, c) {
                    add(&mut names, *t, part);
                }
            }
            [t, 3, 5, f, rel @ ..] if *t >= 128 => {
                let field = fields.entry(*t).or_default().entry(*f).or_default();
                match (rel, c) {
                    ([], Chunk::Kv { key: 2, data }) => field.options = data.to_vec(),
                    ([], Chunk::Kv { key: 3, data }) => field.comment = text(data),
                    ([], Chunk::Kv { key: 9, data }) => field.summary_raw = data.to_vec(),
                    _ => {
                        if let Some((first, s)) = name_part(rel, c) {
                            if first {
                                field.name.clear();
                            }
                            field.name.push_str(&s);
                        }
                    }
                }
            }
            _ => {}
        }
    })?;
    Ok(names
        .into_iter()
        .map(|(id, name)| Table {
            id,
            name,
            fields: fields
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, f)| !f.name.is_empty())
                .collect(),
        })
        .collect())
}

/// Stored values of a table's records, as text (FileMaker stores numbers,
/// dates, times and timestamps as text too, FORMAT.md §13). Keys of a
/// record that are not fields of the table (e.g. 252, a modification
/// count) are left out.
pub fn records(file: &File, table: &Table) -> Result<Records, Error> {
    let t = table.id;
    let mut raw: BTreeMap<u32, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
    // ponytail: repeating-field repetitions (2-byte keys) not split yet.
    file.walk(|path, c| {
        let Some(p) = int_path(path) else { return };
        match (p.as_slice(), c) {
            ([pt, 5, r], Chunk::Kv { key, data }) if *pt == t => {
                raw.entry(*r).or_default().insert(*key, data.to_vec());
            }
            ([pt, 5, r], Chunk::Marker) if *pt == t => {
                raw.entry(*r).or_default();
            }
            // Long value: chunks 1…n, concatenated before decoding (a
            // chunk may end inside an SCSU sequence).
            ([pt, 5, r, f], Chunk::Kv { key, data }) if *pt == t && *key >= 1 => {
                let e = raw.entry(*r).or_default().entry(*f).or_default();
                if *key == 1 {
                    e.clear();
                }
                e.extend_from_slice(data);
            }
            _ => {}
        }
    })?;
    Ok(raw
        .into_iter()
        .map(|(r, vals)| {
            let vals = vals
                .into_iter()
                .filter(|(f, _)| table.fields.contains_key(f))
                .map(|(f, b)| (f, text(&b)));
            (r, vals.collect())
        })
        .collect())
}

/// Order of day, month and year in the file's date format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateOrder {
    #[default]
    Mdy,
    Dmy,
    Ymd,
}

/// The file's saved formats (`[2]` key 13). Records store dates, times,
/// timestamps and numbers as text in these formats (FORMAT.md §13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    /// Language id, as in field option bytes 4–7 (21 = English).
    pub language: u32,
    pub list_separator: char,
    pub thousands: char,
    pub decimal: char,
    pub date_separator: char,
    pub time_separator: char,
    pub date_order: DateOrder,
    /// Suffixes of 12-hour times, e.g. " AM"; empty for a 24-hour locale.
    pub am: String,
    pub pm: String,
}

impl Default for Locale {
    /// English (United States), the format of the English templates.
    fn default() -> Self {
        Locale {
            language: 21,
            list_separator: ',',
            thousands: ',',
            decimal: '.',
            date_separator: '/',
            time_separator: ':',
            date_order: DateOrder::Mdy,
            am: " AM".into(),
            pm: " PM".into(),
        }
    }
}

/// Decodes the locale block: u32 language, 6 bytes, list separator (1 byte),
/// then UTF-16 BE quotes (2), thousands, decimal, date and time separators,
/// `02`, date order (0 MDY, 1 DMY, 2 YMD), 4 format bytes, then the AM and
/// PM strings (length byte + XOR-ed text).
pub fn parse_locale(b: &[u8]) -> Option<Locale> {
    let u16c = |i: usize| char::from_u32(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let mut i = 29;
    let mut pascal = || {
        let n = *b.get(i)? as usize;
        let s = text(b.get(i + 1..i + 1 + n)?);
        i += 1 + n;
        Some(s)
    };
    let (am, pm) = (pascal()?, pascal()?);
    Some(Locale {
        language: u32::from_be_bytes(b.get(..4)?.try_into().ok()?),
        list_separator: *b.get(10)? as char,
        thousands: u16c(15)?,
        decimal: u16c(17)?,
        date_separator: u16c(19)?,
        time_separator: u16c(21)?,
        date_order: match b.get(24)? {
            1 => DateOrder::Dmy,
            2 => DateOrder::Ymd,
            _ => DateOrder::Mdy,
        },
        am,
        pm,
    })
}

/// The file's locale, or English (US) when the block is missing.
pub fn locale(file: &File) -> Result<Locale, Error> {
    let mut out = None;
    file.walk(|path, c| {
        if let (Some([2]), Chunk::Kv { key: 13, data }) = (int_path(path).as_deref(), c) {
            out = parse_locale(data);
        }
    })?;
    Ok(out.unwrap_or_default())
}
