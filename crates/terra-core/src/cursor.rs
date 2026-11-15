//! C# `BinaryReader` / `BinaryWriter` compatible primitives, little-endian.
//!
//! Strings use the same encoding as .NET's `BinaryWriter.Write(string)`:
//! a 7-bit encoded byte-length prefix followed by UTF-8 bytes.

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("unexpected end of data at offset {offset} (needed {needed} more bytes)")]
    Eof { offset: usize, needed: usize },
    #[error("string length {0} exceeds remaining data")]
    BadStringLen(u64),
    #[error("invalid UTF-8 at offset {0}")]
    Utf8(usize),
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], IoError> {
        if self.remaining() < n {
            return Err(IoError::Eof {
                offset: self.pos,
                needed: n,
            });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, IoError> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, IoError> {
        Ok(self.u8()? != 0)
    }

    pub fn i16(&mut self) -> Result<i16, IoError> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u16(&mut self) -> Result<u16, IoError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i32(&mut self) -> Result<i32, IoError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u32(&mut self) -> Result<u32, IoError> {
        Ok(self.i32()? as u32)
    }

    pub fn f32(&mut self) -> Result<f32, IoError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// .NET `BinaryReader.ReadString`: 7-bit length prefix + UTF-8.
    pub fn string(&mut self) -> Result<String, IoError> {
        let start = self.pos;
        let mut len: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.u8()?;
            len |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 63 {
                return Err(IoError::BadStringLen(len));
            }
        }
        if len as usize > self.remaining() {
            return Err(IoError::BadStringLen(len));
        }
        let bytes = self.take(len as usize)?;
        std::str::from_utf8(bytes)
            .map(|s| s.to_owned())
            .map_err(|_| IoError::Utf8(start))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
}

#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }

    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn string(&mut self, s: &str) {
        let b = s.as_bytes();
        let mut len = b.len() as u64;
        loop {
            let byte = (len & 0x7F) as u8;
            len >>= 7;
            if len == 0 {
                self.buf.push(byte);
                break;
            }
            self.buf.push(byte | 0x80);
        }
        self.buf.extend_from_slice(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_string_prefixes() {
        let cases = ["", "a", "TestPlayer", &"x".repeat(300)];
        for s in cases {
            let mut w = Writer::new();
            w.string(s);
            let bytes = w.into_bytes();
            let mut r = Reader::new(&bytes);
            assert_eq!(r.string().unwrap(), s);
            assert!(r.at_end());
        }
        // 300 chars -> length 300 = 0b1_0010_1100 -> LEB128: 0xAC 0x02
        let mut w = Writer::new();
        w.string(&"x".repeat(300));
        assert_eq!(&w.bytes()[..2], &[0xAC, 0x02]);
    }

    #[test]
    fn ints_are_little_endian() {
        let mut w = Writer::new();
        w.i32(-2);
        assert_eq!(w.into_bytes(), vec![0xFE, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn eof_is_reported() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert!(matches!(r.i32(), Err(IoError::Eof { .. })));
    }
}
