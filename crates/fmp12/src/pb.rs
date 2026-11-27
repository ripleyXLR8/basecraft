//! Minimal schema-less Protocol Buffers reader and writer. FileMaker stores
//! layout data (layout header, parts, objects, styles) as protobuf messages.
//!
//! [`Msg`] borrows the bytes it reads; [`MsgBuf`] owns its fields so that an
//! editor can patch some of them and re-encode. Fields keep their wire order
//! and unchanged sub-messages stay as raw bytes, so unknown fields survive
//! byte for byte (FileMaker writes minimal varints, so `encode(parse(b)) == b`).

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

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_field(out: &mut Vec<u8>, field: u32, v: &Value) {
    let (wire, body): (u64, &[u8]) = match v {
        Value::Varint(_) => (0, &[]),
        Value::F64(_) => (1, &[]),
        Value::Bytes(b) => (2, b),
        Value::F32(_) => (5, &[]),
    };
    put_varint(out, (field as u64) << 3 | wire);
    match v {
        Value::Varint(x) => put_varint(out, *x),
        Value::F64(x) => out.extend_from_slice(&x.to_le_bytes()),
        Value::F32(x) => out.extend_from_slice(&x.to_le_bytes()),
        Value::Bytes(_) => {
            put_varint(out, body.len() as u64);
            out.extend_from_slice(body);
        }
    }
}

impl Msg<'_> {
    /// Wire bytes of the message (inverse of [`Msg::parse`]).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (f, v) in &self.0 {
            put_field(&mut out, *f, v);
        }
        out
    }
}

/// An owned field value of a [`MsgBuf`].
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Varint(u64),
    F64(f64),
    F32(f32),
    Bytes(Vec<u8>),
}

/// An owned, editable message: fields in wire order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MsgBuf(pub Vec<(u32, Field)>);

impl MsgBuf {
    pub fn parse(b: &[u8]) -> Option<Self> {
        let m = Msg::parse(b)?;
        Some(MsgBuf(
            m.0.into_iter()
                .map(|(f, v)| {
                    let v = match v {
                        Value::Varint(x) => Field::Varint(x),
                        Value::F64(x) => Field::F64(x),
                        Value::F32(x) => Field::F32(x),
                        Value::Bytes(b) => Field::Bytes(b.to_vec()),
                    };
                    (f, v)
                })
                .collect(),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (f, v) in &self.0 {
            let v = match v {
                Field::Varint(x) => Value::Varint(*x),
                Field::F64(x) => Value::F64(*x),
                Field::F32(x) => Value::F32(*x),
                Field::Bytes(b) => Value::Bytes(b),
            };
            put_field(&mut out, *f, &v);
        }
        out
    }

    pub fn get(&self, field: u32) -> Option<&Field> {
        self.0.iter().find(|(f, _)| *f == field).map(|(_, v)| v)
    }

    pub fn bytes(&self, field: u32) -> Option<&[u8]> {
        match self.get(field)? {
            Field::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Sub-message `field` (an empty one when absent or malformed).
    pub fn msg(&self, field: u32) -> MsgBuf {
        self.bytes(field)
            .and_then(MsgBuf::parse)
            .unwrap_or_default()
    }

    /// Replaces the first occurrence of `field` in place (dropping any
    /// repeats), or appends it.
    pub fn set(&mut self, field: u32, v: Field) {
        match self.0.iter().position(|(f, _)| *f == field) {
            Some(i) => {
                self.0[i].1 = v;
                let mut n = 0;
                self.0.retain(|(f, _)| {
                    n += (*f == field) as usize;
                    *f != field || n == 1
                });
            }
            None => self.0.push((field, v)),
        }
    }

    pub fn set_msg(&mut self, field: u32, m: &MsgBuf) {
        self.set(field, Field::Bytes(m.encode()));
    }

    pub fn remove(&mut self, field: u32) {
        self.0.retain(|(f, _)| *f != field);
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
        assert_eq!(m.encode(), b);
        let mut buf = MsgBuf::parse(&b).unwrap();
        assert_eq!(buf.encode(), b);
        buf.set(1, Field::Varint(300));
        let mut sub = buf.msg(4);
        sub.set(2, Field::F32(0.5));
        buf.set_msg(4, &sub);
        let m = Msg::parse(&buf.encode()).map(|m| m.encode()).unwrap();
        let back = Msg::parse(&m).unwrap();
        assert_eq!(back.uint(1), Some(300));
        assert_eq!(back.str(2), Some("hi"));
        assert_eq!(back.msg(4).unwrap().f64(2), Some(0.5));
        assert_eq!(back.0[0].0, 1, "field order kept");
    }
}
