use crate::{
    TOKENS,
    de::Node,
    error::{Result, bad},
};
use serde::Deserialize;
use std::borrow::Cow;
#[derive(Clone)]
pub struct Cursor<'de> {
    pub(super) bytes: &'de [u8],
    pub(super) pos: usize,
    pub(super) depth: usize,
}
impl<'de> Cursor<'de> {
    pub fn raw_range(&self, start: usize, end: usize) -> &'de [u8] {
        &self.bytes[start..end]
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    pub fn new(bytes: &'de [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            depth: 0,
        }
    }
    pub fn root(bytes: &'de [u8]) -> Result<Self> {
        let mut c = Self::new(bytes);
        if c.tag()? != 10 {
            return Err(bad("root is not a compound"));
        }
        c.name_bytes()?;
        Ok(c)
    }
    pub fn field(&mut self) -> Result<Option<(u8, &'de [u8])>> {
        let t = self.tag()?;
        if t == 0 {
            return Ok(None);
        }
        let key = self.name_bytes()?;
        if !key.is_ascii() {
            cesu8::from_java_cesu8(key).map_err(|_| bad("invalid modified UTF-8"))?;
        }
        if TOKENS.iter().any(|s| s.as_bytes() == key) {
            return Err(bad("reserved array key"));
        }
        Ok(Some((t, key)))
    }
    pub fn field_text(&mut self) -> Result<Option<(u8, Cow<'de, str>)>> {
        let t = self.tag()?;
        if t == 0 {
            return Ok(None);
        }
        let key = self.string()?;
        if TOKENS.contains(&key.as_ref()) {
            return Err(bad("reserved array key"));
        }
        Ok(Some((t, key)))
    }
    pub fn text(&mut self) -> Result<Cow<'de, str>> {
        self.string()
    }
    pub fn value<T: Deserialize<'de>>(&mut self, tag: u8) -> Result<T> {
        T::deserialize(Node { c: self, tag })
    }
    pub fn list_header(&mut self) -> Result<(u8, usize)> {
        let t = self.tag()?;
        let n = self.number()?;
        if !(0..=100_000_000).contains(&n) || (t == 0 && n != 0) {
            return Err(bad("invalid list length"));
        }
        Ok((t, n as usize))
    }

    #[inline]
    pub(super) fn take(&mut self, n: usize) -> Result<&'de [u8]> {
        if n > self.bytes.len() - self.pos {
            return Err(bad("truncated NBT"));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    #[inline]
    pub(super) fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    #[inline]
    pub(super) fn byte(&mut self) -> Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }
    #[inline]
    pub(super) fn tag(&mut self) -> Result<u8> {
        let t = self.byte()?;
        if t > 12 {
            Err(bad("invalid NBT tag"))
        } else {
            Ok(t)
        }
    }
    #[inline]
    pub(super) fn number(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.fixed()?))
    }
    #[inline]
    pub(super) fn name_bytes(&mut self) -> Result<&'de [u8]> {
        let n = u16::from_be_bytes(self.fixed()?) as usize;
        self.take(n)
    }
    #[inline]
    pub(super) fn string(&mut self) -> Result<Cow<'de, str>> {
        cesu8::from_java_cesu8(self.name_bytes()?).map_err(|_| bad("invalid modified UTF-8"))
    }
    pub(super) fn array(&mut self, width: usize) -> Result<&'de [u8]> {
        let n = self.number()?;
        let n = usize::try_from(n).map_err(|_| bad("negative array length"))?;
        self.take(
            n.checked_mul(width)
                .ok_or_else(|| bad("array length overflow"))?,
        )
    }
    #[inline]
    pub(super) fn leaf(&mut self, tag: u8) -> Result<()> {
        match tag {
            1 => {
                self.take(1)?;
            }
            2 => {
                self.take(2)?;
            }
            3 | 5 => {
                self.take(4)?;
            }
            4 | 6 => {
                self.take(8)?;
            }
            7 => {
                self.array(1)?;
            }
            8 => {
                self.name_bytes()?;
            }
            11 => {
                self.array(4)?;
            }
            12 => {
                self.array(8)?;
            }
            _ => return Err(bad("invalid leaf tag")),
        }
        Ok(())
    }
}

pub(super) fn primitive_width(t: u8) -> Option<usize> {
    match t {
        1 => Some(1),
        2 => Some(2),
        3 | 5 => Some(4),
        4 | 6 => Some(8),
        _ => None,
    }
}
