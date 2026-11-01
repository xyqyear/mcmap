use super::Value;
use crate::{ByteArray, IntArray, LongArray, TOKENS, arrays::Raw};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::{collections::HashMap, fmt};

struct ValueVisitor;

macro_rules! number {
    ($method:ident, $ty:ty, $variant:ident) => {
        fn $method<E: de::Error>(self, n: $ty) -> Result<Value, E> {
            Ok(Value::$variant(n))
        }
    };
}

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an NBT value")
    }
    number!(visit_i8, i8, Byte);
    number!(visit_i16, i16, Short);
    number!(visit_i32, i32, Int);
    number!(visit_i64, i64, Long);
    number!(visit_f32, f32, Float);
    number!(visit_f64, f64, Double);
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Value, E> {
        Ok(Value::String(s.into()))
    }
    fn visit_string<E: de::Error>(self, s: String) -> Result<Value, E> {
        Ok(Value::String(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
        while let Some(value) = seq.next_element()? {
            values.push(value);
        }
        Ok(Value::List(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = HashMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if let Some(index) = TOKENS.iter().position(|token| *token == key) {
                if !values.is_empty() {
                    return Err(de::Error::custom("array inside compound"));
                }
                let Raw(bytes) = map.next_value()?;
                let array = match index {
                    0 => Value::ByteArray(ByteArray::from_raw(&bytes)?),
                    1 => Value::IntArray(IntArray::from_raw(&bytes)?),
                    _ => Value::LongArray(LongArray::from_raw(&bytes)?),
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("extra array field"));
                }
                return Ok(array);
            }
            values.insert(key, map.next_value()?);
        }
        Ok(Value::Compound(values))
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}
