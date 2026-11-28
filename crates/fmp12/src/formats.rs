//! Data formatting of a field object: node key 13 of
//! `[4].[5].[L].[5].[obj]` (FORMAT.md §10, "Data formatting"). A fixed
//! record of 210 bytes (213 with the graphic format), big endian, strings
//! in UTF-16 BE padded with zeros. Objects without formats store `00 00`
//! (`00 0C` on new objects of some kinds).
//!
//! Offsets were mapped by pasting layout objects whose clipboard XML
//! (`NumFormat`, `DateFormat`, `TimeFormat` attributes) took one value at a
//! time into FileMaker Pro 26 and reading the bytes it saved; meanings by
//! reading how FileMaker shows a value in Browse mode with each option.

/// Number format option bits (bytes 2–3).
pub mod num {
    /// "General": shown as stored (FileMaker ignores the other options).
    pub const GENERAL: u16 = 0x0001;
    pub const THOUSANDS: u16 = 0x0002;
    /// Currency notation (or percent with [`PERCENT`]).
    pub const NOTATION: u16 = 0x0004;
    pub const FIXED_DIGITS: u16 = 0x0008;
    pub const PERCENT: u16 = 0x0010;
    pub const BOOLEAN: u16 = 0x0020;
    /// "As entered": shown as stored.
    pub const AS_ENTERED: u16 = 0x0100;
    /// Set by FileMaker with currency notation; no visible effect seen.
    pub const CURRENCY: u16 = 0x0200;
    /// The currency symbol goes before the minus sign (`€-1234.57`).
    pub const SYMBOL_OUTSIDE: u16 = 0x0400;
    /// The currency symbol leads (`-€1234.57`); trailing without it.
    pub const LEADING: u16 = 0x0800;
    /// Negative numbers in the negative colour.
    pub const NEGATIVE_COLOR: u16 = 0x1000;
    /// Zero shows as nothing (XML `HideZero`).
    pub const HIDE_ZERO: u16 = 0x2000;
    /// The system's separators (the file's locale) instead of the object's.
    pub const SYSTEM_SEPARATORS: u16 = 0x4000;
}

/// Time format option bits (bytes 150–151).
pub mod time {
    pub const HOURS: u16 = 0x01;
    pub const MINUTES: u16 = 0x02;
    pub const SECONDS: u16 = 0x04;
    /// 24-hour clock; 12-hour with the AM/PM strings without it.
    pub const H24: u16 = 0x08;
    /// Formatted (without it the time shows as entered).
    pub const FORMATTED: u16 = 0x80;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Number {
    pub flags: u16,
    /// Byte 4: numeral style (0 Roman digits).
    pub char_style: u8,
    /// Byte 5: 0 `-1234`, 1 `(1234)`… (see `print::format`).
    pub negative_style: u8,
    pub currency: String,
    pub thousands: char,
    pub decimal: char,
    /// Negative colour, RGB (bytes 22–24).
    pub negative_color: [u8; 3],
    pub digits: u16,
    pub true_text: String,
    pub false_text: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Date {
    /// 0 as entered, 1 custom (elements), 2… predefined (see `print::format`).
    pub format: u8,
    pub char_style: u8,
    pub month_style: u8,
    pub day_style: u8,
    /// Separator of the numeric formats.
    pub separator: char,
    /// Custom format: up to four elements (0 = none).
    pub elements: [u8; 4],
    /// Custom format: text before the first element, between them, after.
    pub separators: [String; 5],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Time {
    pub flags: u16,
    pub char_style: u8,
    pub hour_style: u8,
    pub minsec_style: u8,
    pub separator: char,
    pub am: String,
    pub pm: String,
    pub ampm: String,
}

/// A field object's data formats (equal when the decoded parts are).
#[derive(Debug, Clone, Default)]
pub struct Format {
    pub number: Number,
    pub date: Date,
    pub time: Time,
    /// Container format (bytes 210–211), absent in 210-byte records.
    pub graphic: Option<u16>,
    /// The record as read: unknown bytes are written back unchanged.
    raw: Vec<u8>,
}

impl PartialEq for Format {
    fn eq(&self, o: &Self) -> bool {
        (&self.number, &self.date, &self.time, self.graphic)
            == (&o.number, &o.date, &o.time, o.graphic)
    }
}

/// Length of a full record (with the graphic format).
const LEN: usize = 213;

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn ch(b: &[u8], i: usize) -> char {
    char::from_u32(u16_at(b, i) as u32).unwrap_or('\0')
}

/// UTF-16 BE text of `n` bytes at `i`, up to the first zero unit.
fn text(b: &[u8], i: usize, n: usize) -> String {
    let units: Vec<u16> = b[i..i + n]
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn put_text(b: &mut [u8], i: usize, n: usize, s: &str) {
    b[i..i + n].fill(0);
    for (k, u) in s.encode_utf16().take(n / 2 - 1).enumerate() {
        b[i + 2 * k..i + 2 * k + 2].copy_from_slice(&u.to_be_bytes());
    }
}

fn put_ch(b: &mut [u8], i: usize, c: char) {
    let u = if (c as u32) < 0x10000 {
        c as u32 as u16
    } else {
        0
    };
    b[i..i + 2].copy_from_slice(&u.to_be_bytes());
}

impl Format {
    /// Decodes node key 13; `None` for the 2-byte "no formats" value.
    pub fn parse(b: &[u8]) -> Option<Format> {
        if b.len() < 210 {
            return None;
        }
        let num = Number {
            flags: u16_at(b, 2),
            char_style: b[4],
            negative_style: b[5],
            currency: text(b, 6, 12),
            thousands: ch(b, 18),
            decimal: ch(b, 20),
            negative_color: [b[22], b[23], b[24]],
            digits: u16_at(b, 26),
            true_text: text(b, 28, 16),
            false_text: text(b, 44, 16),
        };
        let date = Date {
            format: b[60],
            char_style: b[61],
            month_style: b[62],
            day_style: b[63],
            separator: ch(b, 64),
            elements: [b[66], b[67], b[68], b[69]],
            separators: std::array::from_fn(|k| text(b, 70 + 16 * k, 16)),
        };
        let time = Time {
            flags: u16_at(b, 150),
            char_style: b[152],
            hour_style: b[153],
            minsec_style: b[154],
            separator: ch(b, 156),
            am: text(b, 158, 16),
            pm: text(b, 174, 16),
            ampm: text(b, 190, 16),
        };
        Some(Format {
            number: num,
            date,
            time,
            graphic: (b.len() >= 212).then(|| u16_at(b, 210)),
            raw: b.to_vec(),
        })
    }

    /// FileMaker's formats for a new field object (as on the templates' edit
    /// boxes: general numbers, dates and times as entered, graphics 5).
    pub fn new_default() -> Format {
        let mut b = vec![0u8; LEN];
        b[2..4].copy_from_slice(&0x0900u16.to_be_bytes());
        put_ch(&mut b, 6, '\0');
        b[7] = b'#';
        put_ch(&mut b, 18, ',');
        put_ch(&mut b, 20, '.');
        b[22] = 0xDD;
        put_text(&mut b, 28, 16, "Yes");
        put_text(&mut b, 44, 16, "No");
        b[150..152].copy_from_slice(&0x008Fu16.to_be_bytes());
        put_ch(&mut b, 156, ':');
        put_text(&mut b, 158, 16, " AM");
        put_text(&mut b, 174, 16, " PM");
        b[210..212].copy_from_slice(&5u16.to_be_bytes());
        Format::parse(&b).unwrap_or_default()
    }

    /// The record, unknown bytes as read.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = self.raw.clone();
        let len = if self.graphic.is_some() { LEN } else { 210 };
        b.resize(len.max(b.len()), 0);
        let n = &self.number;
        b[2..4].copy_from_slice(&n.flags.to_be_bytes());
        b[4] = n.char_style;
        b[5] = n.negative_style;
        put_text(&mut b, 6, 12, &n.currency);
        put_ch(&mut b, 18, n.thousands);
        put_ch(&mut b, 20, n.decimal);
        b[22..25].copy_from_slice(&n.negative_color);
        b[26..28].copy_from_slice(&n.digits.to_be_bytes());
        put_text(&mut b, 28, 16, &n.true_text);
        put_text(&mut b, 44, 16, &n.false_text);
        let d = &self.date;
        b[60] = d.format;
        b[61] = d.char_style;
        b[62] = d.month_style;
        b[63] = d.day_style;
        put_ch(&mut b, 64, d.separator);
        b[66..70].copy_from_slice(&d.elements);
        for (k, s) in d.separators.iter().enumerate() {
            put_text(&mut b, 70 + 16 * k, 16, s);
        }
        let t = &self.time;
        b[150..152].copy_from_slice(&t.flags.to_be_bytes());
        b[152] = t.char_style;
        b[153] = t.hour_style;
        b[154] = t.minsec_style;
        put_ch(&mut b, 156, t.separator);
        put_text(&mut b, 158, 16, &t.am);
        put_text(&mut b, 174, 16, &t.pm);
        put_text(&mut b, 190, 16, &t.ampm);
        if let Some(g) = self.graphic {
            b[210..212].copy_from_slice(&g.to_be_bytes());
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_record_round_trips() {
        let f = Format::new_default();
        let b = f.encode();
        assert_eq!(b.len(), LEN);
        assert_eq!(Format::parse(&b), Some(f.clone()));
        assert_eq!(f.number.flags, 0x0900);
        assert_eq!(f.number.true_text, "Yes");
        assert_eq!(f.time.pm, " PM");
        assert_eq!(f.graphic, Some(5));
        let mut g = f;
        g.number.currency = "€".into();
        g.date.separators[1] = "/".into();
        let b = g.encode();
        // Seen in FileMaker's file: "€" at bytes 6–7, separator 1 at 86.
        assert_eq!(&b[6..8], &[0x20, 0xAC]);
        assert_eq!(&b[86..88], &[0, b'/']);
        assert_eq!(Format::parse(&b), Some(g));
        assert_eq!(Format::parse(&[0, 0]), None);
    }
}
