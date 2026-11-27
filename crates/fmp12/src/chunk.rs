//! Byte-code decoder for the payload of an HBAM7 sector.
//!
//! See `docs/FORMAT.md` for the opcode table.

use crate::Error;

/// One decoded unit of a sector payload. Byte slices borrow the sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunk<'a> {
    /// Anonymous data at the current path.
    Data(&'a [u8]),
    /// Integer key → value.
    Kv { key: u32, data: &'a [u8] },
    /// Byte-string key → value.
    KvLong { key: &'a [u8], data: &'a [u8] },
    /// Enter a sub-path.
    Push(&'a [u8]),
    /// Leave the current sub-path.
    Pop,
    /// Op 0x80: the current path exists as a node. It sorts before
    /// everything under that path.
    Marker,
}

/// Decodes a "path integer" (1 to 3 bytes).
pub fn path_int(b: &[u8]) -> Option<u32> {
    match b {
        [a] => Some(*a as u32),
        [a, b] => Some(0x80 + (((*a & 0x7F) as u32) << 8) + *b as u32),
        [_, b, c] => Some(0x80 + ((*b as u32) << 8) + *c as u32),
        _ => None,
    }
}

fn be(b: &[u8]) -> u32 {
    b.iter().fold(0, |acc, x| (acc << 8) | *x as u32)
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.buf.len());
        let end = end.ok_or(Error::Truncated(self.pos))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }
}

/// Decodes a whole sector payload into chunks.
pub fn parse(payload: &[u8]) -> Result<Vec<Chunk<'_>>, Error> {
    Ok(parse_spans(payload)?.into_iter().map(|(c, _)| c).collect())
}

/// Like [`parse`], with the bytes each chunk was decoded from (opcode
/// included, pop flag kept). The pop implied by a flag has an empty span.
pub fn parse_spans(payload: &[u8]) -> Result<Vec<(Chunk<'_>, &[u8])>, Error> {
    let mut c = Cursor {
        buf: payload,
        pos: 0,
    };
    let mut out = Vec::new();
    while c.pos < payload.len() {
        let at = c.pos;
        let raw = c.byte()?;
        // Codes with both top bits set: decode as `raw & 0x3F`, then pop.
        let pop_after = raw & 0xC0 == 0xC0;
        let op = if pop_after { raw & 0x3F } else { raw };
        let chunk = match op {
            0x00 => match c.peek() {
                None | Some(0) => break, // end of payload
                Some(_) => Chunk::Data(c.take(1)?),
            },
            0x01..=0x05 => {
                let key = c.byte()? as u32;
                let n = if op == 1 { 1 } else { 2 * (op as usize - 1) };
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            0x06 => {
                let key = c.byte()? as u32;
                let n = c.byte()? as usize;
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            // Long forms of 0x06 / 0x0E: 2-byte length.
            0x07 => {
                let key = c.byte()? as u32;
                let n = be(c.take(2)?) as usize;
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            0x08 => Chunk::Data(c.take(2)?),
            0x0E if c.peek() == Some(0xFF) => Chunk::Data(c.take(6)?),
            0x09..=0x0D => {
                let key = path_int(c.take(2)?).unwrap();
                let n = if op == 9 { 1 } else { 2 * (op as usize - 9) };
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            0x0E => {
                let key = path_int(c.take(2)?).unwrap();
                let n = c.byte()? as usize;
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            0x0F => {
                let key = path_int(c.take(2)?).unwrap();
                let n = be(c.take(2)?) as usize;
                Chunk::Kv {
                    key,
                    data: c.take(n)?,
                }
            }
            0x10 => Chunk::Data(c.take(3)?),
            0x11..=0x15 => {
                let n = 3 + (op == 0x11) as usize + 2 * (op as usize - 0x11);
                Chunk::Data(c.take(n)?)
            }
            0x16 => {
                let key = c.take(3)?;
                let n = c.byte()? as usize;
                Chunk::KvLong {
                    key,
                    data: c.take(n)?,
                }
            }
            0x17 => {
                let key = c.take(3)?;
                let n = be(c.take(2)?) as usize;
                Chunk::KvLong {
                    key,
                    data: c.take(n)?,
                }
            }
            0x18 | 0x23 => {
                let n = c.byte()? as usize;
                Chunk::Data(c.take(n)?)
            }
            0x1B if c.peek() == Some(0) => {
                c.take(1)?;
                let key = c.byte()? as u32;
                Chunk::Kv {
                    key,
                    data: c.take(4)?,
                }
            }
            // Length-prefixed key, fixed-size value (like 0x01–0x05).
            0x19..=0x1D => {
                let k = c.byte()? as usize;
                let key = c.take(k)?;
                let n = if op == 0x19 {
                    1
                } else {
                    2 * (op as usize - 0x19)
                };
                Chunk::KvLong {
                    key,
                    data: c.take(n)?,
                }
            }
            0x1E => {
                let k = c.byte()? as usize;
                let key = c.take(k)?;
                let n = c.byte()? as usize;
                Chunk::KvLong {
                    key,
                    data: c.take(n)?,
                }
            }
            0x1F => {
                let k = c.byte()? as usize;
                let key = c.take(k)?;
                let n = be(c.take(2)?) as usize;
                Chunk::KvLong {
                    key,
                    data: c.take(n)?,
                }
            }
            0x20 => {
                if c.peek() == Some(0xFE) {
                    c.take(1)?;
                    Chunk::Push(c.take(8)?)
                } else {
                    Chunk::Push(c.take(1)?)
                }
            }
            0x28 => Chunk::Push(c.take(2)?),
            0x30 => Chunk::Push(c.take(3)?),
            0x38 => {
                let n = c.byte()? as usize;
                Chunk::Push(c.take(n)?)
            }
            0x3D | 0x40 => Chunk::Pop,
            0x80 => Chunk::Marker,
            0x88 => continue, // no-op
            _ => return Err(Error::UnknownOpcode { op: raw, at }),
        };
        out.push((chunk, &payload[at..c.pos]));
        if pop_after {
            out.push((Chunk::Pop, &payload[c.pos..c.pos]));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_basic_ops() {
        // push 3, kv key 16 = 1 byte, segment 1 of 2 bytes, pop, push+pop (0xE0)
        let p = [
            0x20, 3, 0x01, 16, 0xAA, 0x07, 1, 0, 2, 9, 9, 0x40, 0xE0, 5, 0, 0,
        ];
        let c = parse(&p).unwrap();
        assert_eq!(
            c,
            vec![
                Chunk::Push(&[3]),
                Chunk::Kv {
                    key: 16,
                    data: &[0xAA]
                },
                Chunk::Kv {
                    key: 1,
                    data: &[9, 9]
                },
                Chunk::Pop,
                Chunk::Push(&[5]),
                Chunk::Pop,
            ]
        );
    }

    #[test]
    fn path_ints() {
        assert_eq!(path_int(&[5]), Some(5));
        assert_eq!(path_int(&[0x80, 0x02]), Some(0x82));
        assert_eq!(path_int(&[0x81, 0x00]), Some(0x180));
        assert_eq!(path_int(&[0xFF, 0x00, 0x01]), Some(0x81));
    }

    #[test]
    fn truncated_is_error() {
        assert!(matches!(parse(&[0x06, 1, 5, 0]), Err(Error::Truncated(_))));
    }
}
