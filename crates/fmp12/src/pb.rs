//! Minimal schema-less Protocol Buffers reader. FileMaker stores layout
//! data (layout header, parts, objects, styles) as protobuf messages.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Varint(u64),
    F64(f64),
    F32(f32),
    Bytes(&'a [u8]),
}

/// A decoded message: fields in wire order (repeated fields appear several times).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Msg<'a>(pub Vec<(u32, Value<'a>)>);

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut r = 0u64;
    for shift in (0..64).step_by(7) {
        let x = *b.get(*i)?;
        *i += 1;
        r |= ((x & 0x7F) as u64) << shift;
        if x < 0x80 {
            return Some(r);
        }
    }
    None
}

impl<'a> Msg<'a> {
    /// Parses `b`; `None` if it is not a well-formed message.
    pub fn parse(b: &'a [u8]) -> Option<Self> {
        let mut i = 0;
        let mut out = Vec::new();
        while i < b.len() {
            let key = varint(b, &mut i)?;
            let field = u32::try_from(key >> 3).ok()?;
            let v = match key & 7 {
                0 => Value::Varint(varint(b, &mut i)?),
                1 => {
                    let x = b.get(i..i + 8)?;
                    i += 8;
                    Value::F64(f64::from_le_bytes(x.try_into().ok()?))
                }
                2 => {
                    let n = varint(b, &mut i)? as usize;
                    let x = b.get(i..i.checked_add(n)?)?;
                    i += n;
                    Value::Bytes(x)
                }
                5 => {
                    let x = b.get(i..i + 4)?;
                    i += 4;
                    Value::F32(f32::from_le_bytes(x.try_into().ok()?))
                }
                _ => return None,
            };
            out.push((field, v));
        }
        Some(Msg(out))
    }

    pub fn all(&self, field: u32) -> impl Iterator<Item = &Value<'a>> {
        self.0
            .iter()
            .filter(move |(f, _)| *f == field)
            .map(|(_, v)| v)
    }

    pub fn get(&self, field: u32) -> Option<&Value<'a>> {
        self.all(field).next()
    }

    pub fn uint(&self, field: u32) -> Option<u64> {
        match self.get(field)? {
            Value::Varint(v) => Some(*v),
            _ => None,
        }
    }

    pub fn f64(&self, field: u32) -> Option<f64> {
        match self.get(field)? {
            Value::F64(v) => Some(*v),
            Value::F32(v) => Some(*v as f64),
            Value::Varint(v) => Some(*v as f64),
            Value::Bytes(_) => None,
        }
    }

    pub fn bytes(&self, field: u32) -> Option<&'a [u8]> {
        match self.get(field)? {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn str(&self, field: u32) -> Option<&'a str> {
        std::str::from_utf8(self.bytes(field)?).ok()
    }

    pub fn msg(&self, field: u32) -> Option<Msg<'a>> {
        Msg::parse(self.bytes(field)?)
    }

    pub fn msgs(&self, field: u32) -> impl Iterator<Item = Msg<'a>> + '_ {
        self.all(field).filter_map(|v| match v {
            Value::Bytes(b) => Msg::parse(b),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wire_types() {
        // 1: 150 (varint), 2: "hi", 3: 1.5 (double), 4: { 1: 7 }
        let mut b = vec![0x08, 0x96, 0x01, 0x12, 2, b'h', b'i', 0x19];
        b.extend_from_slice(&1.5f64.to_le_bytes());
        b.extend_from_slice(&[0x22, 2, 0x08, 7]);
        let m = Msg::parse(&b).unwrap();
        assert_eq!(m.uint(1), Some(150));
        assert_eq!(m.str(2), Some("hi"));
        assert_eq!(m.f64(3), Some(1.5));
        assert_eq!(m.msg(4).unwrap().uint(1), Some(7));
        assert!(Msg::parse(&[0x12, 5, 1]).is_none());
    }
}
