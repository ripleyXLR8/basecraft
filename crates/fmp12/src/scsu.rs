//! Decoder for SCSU (Unicode Technical Standard #6), FileMaker's text encoding.
//! Control characters (CR, TAB…) are kept verbatim: FileMaker uses CR as its
//! paragraph separator (¶).

const STATIC: [u32; 8] = [
    0x0000, 0x0080, 0x0100, 0x0300, 0x2000, 0x2080, 0x2100, 0x3000,
];
const DYNAMIC: [u32; 8] = [
    0x0080, 0x00C0, 0x0400, 0x0600, 0x0900, 0x3040, 0x30A0, 0xFF00,
];

fn window_offset(x: u8) -> u32 {
    match x {
        0x01..=0x67 => x as u32 * 0x80,
        0x68..=0xA7 => x as u32 * 0x80 + 0xAC00,
        0xF9 => 0x00C0,
        0xFA => 0x0250,
        0xFB => 0x0370,
        0xFC => 0x0530,
        0xFD => 0x3040,
        0xFE => 0x30A0,
        0xFF => 0xFF60,
        _ => 0, // reserved
    }
}

fn extended(h: u8, l: u8) -> u32 {
    0x10000 + 0x80 * ((((h & 0x1F) as u32) << 8) + l as u32)
}

/// Decodes SCSU bytes. Invalid or truncated input yields U+FFFD, never a panic.
pub fn decode(src: &[u8]) -> String {
    let mut dynamic = DYNAMIC;
    let mut window = 0usize;
    let mut unicode_mode = false;
    let mut units: Vec<u16> = Vec::with_capacity(src.len());
    let mut i = 0;
    let get = |i: usize| src.get(i).copied();
    let push = |u: u32, units: &mut Vec<u16>| {
        if u >= 0x10000 {
            let v = u - 0x10000;
            units.push(0xD800 | (v >> 10) as u16);
            units.push(0xDC00 | (v & 0x3FF) as u16);
        } else {
            units.push(u as u16);
        }
    };
    while i < src.len() {
        let c = src[i];
        i += 1;
        if unicode_mode {
            match c {
                0xE0..=0xE7 => {
                    window = (c - 0xE0) as usize;
                    unicode_mode = false;
                }
                0xE8..=0xEF => {
                    window = (c - 0xE8) as usize;
                    dynamic[window] = window_offset(get(i).unwrap_or(0));
                    i += 1;
                    unicode_mode = false;
                }
                0xF1 => {
                    let (h, l) = (get(i).unwrap_or(0), get(i + 1).unwrap_or(0));
                    window = ((h & 0xE0) >> 5) as usize;
                    dynamic[window] = extended(h, l);
                    i += 2;
                    unicode_mode = false;
                }
                0xF0 => {
                    let u =
                        ((get(i).unwrap_or(0xFF) as u16) << 8) | get(i + 1).unwrap_or(0xFD) as u16;
                    units.push(u);
                    i += 2;
                }
                _ => {
                    units.push(((c as u16) << 8) | get(i).unwrap_or(0xFD) as u16);
                    i += 1;
                }
            }
            continue;
        }
        match c {
            0x01..=0x08 => {
                let w = (c - 1) as usize;
                let b = get(i).unwrap_or(0);
                i += 1;
                let u = if b < 0x80 {
                    STATIC[w] + b as u32
                } else {
                    dynamic[w] + (b - 0x80) as u32
                };
                push(u, &mut units);
            }
            0x0B => {
                let (h, l) = (get(i).unwrap_or(0), get(i + 1).unwrap_or(0));
                window = ((h & 0xE0) >> 5) as usize;
                dynamic[window] = extended(h, l);
                i += 2;
            }
            0x0E => {
                units.push(
                    ((get(i).unwrap_or(0xFF) as u16) << 8) | get(i + 1).unwrap_or(0xFD) as u16,
                );
                i += 2;
            }
            0x0F => unicode_mode = true,
            0x10..=0x17 => window = (c - 0x10) as usize,
            0x18..=0x1F => {
                window = (c - 0x18) as usize;
                dynamic[window] = window_offset(get(i).unwrap_or(0));
                i += 1;
            }
            0x80..=0xFF => push(dynamic[window] + (c - 0x80) as u32, &mut units),
            _ => units.push(c as u16), // ASCII and NUL/TAB/LF/CR pass through
        }
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn ascii_and_latin1() {
        assert_eq!(decode(b"Caf\xE9\r2"), "Café\r2");
    }

    #[test]
    fn windows_and_unicode_mode() {
        // SC2 → Cyrillic window, 0x80+0x10 = U+0410 'А'
        assert_eq!(decode(&[0x12, 0x90]), "А");
        // SCU then U+65E5 '日' then UC0 back, 'a'
        assert_eq!(decode(&[0x0F, 0x65, 0xE5, 0xE0, b'a']), "日a");
        // SQU quote U+20AC
        assert_eq!(decode(&[0x0E, 0x20, 0xAC]), "€");
    }
}
