use crate::{
    Cursor, Error, TOKENS,
    de::Node,
    error::{Result, bad},
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess};
use std::borrow::Cow;
pub(super) struct Compound<'a, 'de> {
    pub(super) c: &'a mut Cursor<'de>,
    pub(super) tag: u8,
}
impl<'de> MapAccess<'de> for Compound<'_, 'de> {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        self.tag = self.c.tag()?;
        if self.tag == 0 {
            return Ok(None);
        }
        let key = self.c.string()?;
        if key.starts_with("__") && TOKENS.contains(&key.as_ref()) {
            return Err(bad("reserved array key"));
        }
        match key {
            Cow::Borrowed(s) => {
                seed.deserialize(de::value::BorrowedStrDeserializer::<Error>::new(s))
            }
            Cow::Owned(s) => seed.deserialize(de::value::StrDeserializer::<Error>::new(&s)),
        }
        .map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        seed.deserialize(Node {
            c: self.c,
            tag: self.tag,
        })
    }
}
pub(super) struct List<'a, 'de> {
    pub(super) c: &'a mut Cursor<'de>,
    pub(super) tag: u8,
    pub(super) left: usize,
}
impl<'de> SeqAccess<'de> for List<'_, 'de> {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        seed.deserialize(Node {
            c: self.c,
            tag: self.tag,
        })
        .map(Some)
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}
pub(super) struct Array<'de> {
    pub(super) key: Option<&'static str>,
    pub(super) bytes: &'de [u8],
}
impl<'de> MapAccess<'de> for Array<'de> {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        self.key
            .take()
            .map(|s| seed.deserialize(de::value::BorrowedStrDeserializer::<Error>::new(s)))
            .transpose()
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        seed.deserialize(de::value::BorrowedBytesDeserializer::<Error>::new(
            self.bytes,
        ))
    }
}
