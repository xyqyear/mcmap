use crate::{
    Cursor, Error, TOKENS,
    access::{Array, Compound, List},
    error::{Result, bad},
};
use serde::de::{self, DeserializeSeed, Visitor};
use serde::{Deserialize, Deserializer, forward_to_deserialize_any};
use std::borrow::Cow;
pub fn from_bytes<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T> {
    let mut c = Cursor::new(bytes);
    if c.tag()? != 10 {
        return Err(bad("root is not a compound"));
    }
    c.name_bytes()?;
    T::deserialize(Node { c: &mut c, tag: 10 })
}
pub(crate) struct Node<'a, 'de> {
    pub(crate) c: &'a mut Cursor<'de>,
    pub(crate) tag: u8,
}
impl<'de> Deserializer<'de> for Node<'_, 'de> {
    type Error = Error;
    forward_to_deserialize_any! { u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 str string struct tuple map identifier char }
    fn deserialize_any<V: Visitor<'de>>(mut self, v: V) -> Result<V::Value> {
        if matches!(self.tag, 9 | 10) {
            if self.c.depth >= 512 {
                return Err(bad("NBT nesting exceeds 512 containers"));
            }
            self.c.depth += 1;
            let result = self.container(v);
            self.c.depth -= 1;
            return result;
        }
        match self.tag {
            1 => v.visit_i8(self.c.byte()? as i8),
            2 => v.visit_i16(i16::from_be_bytes(self.c.fixed()?)),
            3 => v.visit_i32(self.c.number()?),
            4 => v.visit_i64(i64::from_be_bytes(self.c.fixed()?)),
            5 => v.visit_f32(f32::from_be_bytes(self.c.fixed()?)),
            6 => v.visit_f64(f64::from_be_bytes(self.c.fixed()?)),
            8 => match self.c.string()? {
                Cow::Borrowed(s) => v.visit_borrowed_str(s),
                Cow::Owned(s) => v.visit_str(&s),
            },
            7 | 11 | 12 => {
                let index = match self.tag {
                    7 => 0,
                    11 => 1,
                    _ => 2,
                };
                let width = [1, 4, 8][index];
                v.visit_map(Array {
                    key: Some(TOKENS[index]),
                    bytes: self.c.array(width)?,
                })
            }
            _ => Err(bad("unexpected end tag")),
        }
    }
    fn deserialize_seq<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if matches!(self.tag, 7 | 11 | 12) {
            return Err(bad("array cannot be a list"));
        }
        self.deserialize_any(v)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        v: V,
    ) -> Result<V::Value> {
        self.deserialize_seq(v)
    }
    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_some(self)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_newtype_struct(self)
    }
    fn deserialize_unit<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        self.c.skip(self.tag)?;
        v.visit_unit()
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, v: V) -> Result<V::Value> {
        self.deserialize_unit(v)
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        self.deserialize_unit(v)
    }
    fn deserialize_bool<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self.tag {
            1 => v.visit_bool(self.c.byte()? != 0),
            2 => v.visit_bool(i16::from_be_bytes(self.c.fixed()?) != 0),
            3 => v.visit_bool(self.c.number()? != 0),
            4 => v.visit_bool(i64::from_be_bytes(self.c.fixed()?) != 0),
            _ => self.deserialize_any(v),
        }
    }
    fn deserialize_bytes<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        let b = match self.tag {
            8 => self.c.name_bytes()?,
            7 => self.c.array(1)?,
            12 => self.c.array(8)?,
            9 => {
                let tag = self.c.tag()?;
                let width = match tag {
                    1 => 1,
                    2 => 2,
                    3 => 4,
                    4 => 8,
                    _ => return Err(bad("list cannot be bytes")),
                };
                self.c.array(width)?
            }
            _ => return Err(bad("tag cannot be bytes")),
        };
        v.visit_borrowed_bytes(b)
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        self.deserialize_bytes(v)
    }
    fn deserialize_i128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if self.tag != 11 {
            return Err(bad("i128 needs int array"));
        }
        let b = self.c.array(4)?;
        v.visit_i128(i128::from_be_bytes(
            b.try_into().map_err(|_| bad("i128 needs 16 bytes"))?,
        ))
    }
    fn deserialize_u128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if self.tag != 11 {
            return Err(bad("u128 needs int array"));
        }
        let b = self.c.array(4)?;
        v.visit_u128(u128::from_be_bytes(
            b.try_into().map_err(|_| bad("u128 needs 16 bytes"))?,
        ))
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        v.visit_enum(Enum(self))
    }
}
impl<'de> Node<'_, 'de> {
    fn container<V: Visitor<'de>>(&mut self, visitor: V) -> Result<V::Value> {
        if self.tag == 10 {
            visitor.visit_map(Compound { c: self.c, tag: 0 })
        } else {
            let (tag, left) = self.c.list_header()?;
            visitor.visit_seq(List {
                c: self.c,
                tag,
                left,
            })
        }
    }
}
struct Enum<'a, 'de>(Node<'a, 'de>);
impl<'de> de::EnumAccess<'de> for Enum<'_, 'de> {
    type Error = Error;
    type Variant = Self;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        let n = self.0;
        let value = seed.deserialize(Node {
            c: &mut *n.c,
            tag: n.tag,
        })?;
        Ok((value, Self(n)))
    }
}
impl<'de> de::VariantAccess<'de> for Enum<'_, 'de> {
    type Error = Error;
    fn unit_variant(self) -> Result<()> {
        Ok(())
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, _: T) -> Result<T::Value> {
        Err(bad("unsupported enum payload"))
    }
    fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value> {
        Err(bad("unsupported enum payload"))
    }
    fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], _: V) -> Result<V::Value> {
        Err(bad("unsupported enum payload"))
    }
}
