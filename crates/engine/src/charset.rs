//! Character sets of Import Records' and Export Records' "Character set"
//! lists: UTF-8, UTF-16 (with a byte order mark), Windows ANSI (code page
//! 1252), Macintosh (Mac OS Roman) and ASCII, by tables in code.
//!
//! As FileMaker documents it, a byte order mark decides the encoding of a
//! source file (the list is dimmed), and UTF-16 is only recognized with one.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Charset {
    #[default]
    Utf8,
    /// Little endian with a byte order mark on export.
    Utf16,
    /// Windows code page 1252 (a superset of ISO-8859-1).
    WindowsAnsi,
    /// Mac OS Roman.
    MacRoman,
    /// 7-bit ASCII; other characters export as `?`.
    // ponytail: FileMaker's "ASCII (DOS)" is likely an OEM code page (437);
    // only 7-bit ASCII is mapped.
    Ascii,
}

impl Charset {
    pub const ALL: [Charset; 5] = [
        Charset::Utf8,
        Charset::Utf16,
        Charset::WindowsAnsi,
        Charset::MacRoman,
        Charset::Ascii,
    ];

    /// The name FileMaker's list shows.
    pub fn label(self) -> &'static str {
        match self {
            Charset::Utf8 => "Unicode (UTF-8)",
            Charset::Utf16 => "Unicode (UTF-16)",
            Charset::WindowsAnsi => "Windows (ANSI)",
            Charset::MacRoman => "Macintosh",
            Charset::Ascii => "ASCII (DOS)",
        }
    }

    /// From a name: `utf-8`, `utf-16`, `windows`/`ansi`/`latin1`/`cp1252`,
    /// `mac`/`macroman`, `ascii`/`dos`, or a [`Charset::label`].
    pub fn from_name(s: &str) -> Option<Charset> {
        let k: String = s
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        Some(match k.as_str() {
            "utf8" | "unicodeutf8" => Charset::Utf8,
            "utf16" | "unicodeutf16" | "unicode" => Charset::Utf16,
            "windows" | "ansi" | "windowsansi" | "latin1" | "iso88591" | "cp1252"
            | "windows1252" => Charset::WindowsAnsi,
            "mac" | "macintosh" | "macroman" => Charset::MacRoman,
            "ascii" | "asciidos" | "dos" => Charset::Ascii,
            _ => return None,
        })
    }
}

/// Code page 1252, bytes 0x80–0x9F (unassigned bytes as C1 controls).
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Mac OS Roman, bytes 0x80–0xFF.
const MAC_ROMAN: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
    '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

fn decode_byte(b: u8, cs: Charset) -> char {
    match (cs, b) {
        (_, 0..=0x7F) => b as char,
        (Charset::MacRoman, _) => MAC_ROMAN[(b - 0x80) as usize],
        (Charset::WindowsAnsi, 0x80..=0x9F) => CP1252[(b - 0x80) as usize],
        (Charset::Ascii, _) => '\u{FFFD}',
        _ => b as char,
    }
}

/// Text of a source file in `cs`; a byte order mark (UTF-8, UTF-16 LE or
/// BE) overrides `cs`. Invalid sequences become U+FFFD.
pub fn decode(bytes: &[u8], cs: Charset) -> String {
    let utf16 = |b: &[u8], le: bool| {
        let units: Vec<u16> = b
            .chunks(2)
            .map(|c| {
                let pair = [c[0], c.get(1).copied().unwrap_or(0)];
                if le {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(b) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(b).into_owned();
    }
    if let Some(b) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(b, true);
    }
    if let Some(b) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(b, false);
    }
    match cs {
        Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        // Without a byte order mark: little endian.
        Charset::Utf16 => utf16(bytes, true),
        _ => bytes.iter().map(|&b| decode_byte(b, cs)).collect(),
    }
}

/// `s` in `cs` (UTF-16: little endian with a byte order mark). Characters
/// the set cannot hold become `?`.
pub fn encode(s: &str, cs: Charset) -> Vec<u8> {
    match cs {
        Charset::Utf8 => s.as_bytes().to_vec(),
        Charset::Utf16 => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        _ => s.chars().map(|c| encode_char(c, cs)).collect(),
    }
}

fn encode_char(c: char, cs: Charset) -> u8 {
    if (c as u32) < 0x80 {
        return c as u8;
    }
    let table: &[char] = match cs {
        Charset::MacRoman => &MAC_ROMAN,
        Charset::WindowsAnsi => &CP1252,
        _ => &[],
    };
    if let Some(i) = table.iter().position(|&x| x == c) {
        return 0x80 + i as u8;
    }
    match (cs, c as u32) {
        (Charset::WindowsAnsi, 0xA0..=0xFF) => c as u32 as u8,
        _ => b'?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_boms() {
        let s = "Café “déjà” €5 – Ω";
        for cs in [Charset::Utf8, Charset::Utf16, Charset::MacRoman] {
            assert_eq!(decode(&encode(s, cs), cs), s, "{cs:?}");
        }
        // Ω is not in code page 1252.
        assert_eq!(
            decode(&encode(s, Charset::WindowsAnsi), Charset::WindowsAnsi),
            "Café “déjà” €5 – ?"
        );
        assert_eq!(encode("é€", Charset::WindowsAnsi), [0xE9, 0x80]);
        assert_eq!(encode("é€", Charset::MacRoman), [0x8E, 0xDB]);
        assert_eq!(encode("é", Charset::Ascii), b"?");
        // A byte order mark wins over the chosen set.
        assert_eq!(decode(b"\xFE\xFF\x00A\x00\xE9", Charset::MacRoman), "Aé");
        assert_eq!(decode(b"\xEF\xBB\xBFx", Charset::Ascii), "x");
        assert_eq!(Charset::from_name("Mac Roman"), Some(Charset::MacRoman));
        assert_eq!(Charset::from_name("latin-1"), Some(Charset::WindowsAnsi));
    }
}
