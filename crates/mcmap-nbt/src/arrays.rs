use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    borrow::Cow,
    fmt,
    ops::{Deref, DerefMut},
};

pub(crate) struct Raw<'a>(pub Cow<'a, [u8]>);

impl<'de> Deserialize<'de> for Raw<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Bytes;
        impl<'de> Visitor<'de> for Bytes {
            type Value = Raw<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("raw array bytes")
            }
            fn visit_borrowed_bytes<E: de::Error>(
                self,
                bytes: &'de [u8],
            ) -> Result<Self::Value, E> {
                Ok(Raw(Cow::Borrowed(bytes)))
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Self::Value, E> {
                Ok(Raw(Cow::Owned(bytes.to_vec())))
            }
            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Self::Value, E> {
                Ok(Raw(Cow::Owned(bytes)))
            }
        }
        d.deserialize_bytes(Bytes)
    }
}

macro_rules! array {
    ($name:ident, $element:ty, $width:literal, $index:literal) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            data: Vec<$element>,
        }

        impl $name {
            pub fn new(data: Vec<$element>) -> Self {
                Self { data }
            }
            pub fn into_inner(self) -> Vec<$element> {
                self.data
            }
            pub(crate) fn from_raw<E: de::Error>(bytes: &[u8]) -> Result<Self, E> {
                if bytes.len() % $width != 0 {
                    return Err(E::custom("unaligned array length"));
                }
                let data = bytes
                    .chunks_exact($width)
                    .map(|b| <$element>::from_be_bytes(b.try_into().unwrap()))
                    .collect();
                Ok(Self { data })
            }
        }
        impl Deref for $name {
            type Target = [$element];
            fn deref(&self) -> &Self::Target {
                &self.data
            }
        }
        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.data
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_newtype_struct(crate::TOKENS[$index], &self.data)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct ArrayVisitor;
                impl<'de> Visitor<'de> for ArrayVisitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        f.write_str(stringify!($name))
                    }
                    fn visit_map<A: MapAccess<'de>>(
                        self,
                        mut map: A,
                    ) -> Result<Self::Value, A::Error> {
                        let key = map
                            .next_key::<&str>()?
                            .ok_or_else(|| de::Error::custom("missing array type"))?;
                        if key != crate::TOKENS[$index] {
                            return Err(de::Error::custom("wrong array type"));
                        }
                        let Raw(bytes) = map.next_value()?;
                        let array = $name::from_raw(&bytes)?;
                        if map.next_key::<de::IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom("extra array field"));
                        }
                        Ok(array)
                    }
                }
                d.deserialize_map(ArrayVisitor)
            }
        }
    };
}

array!(ByteArray, i8, 1, 0);
array!(IntArray, i32, 4, 1);
array!(LongArray, i64, 8, 2);
