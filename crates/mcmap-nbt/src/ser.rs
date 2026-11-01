mod compound;
mod sequence;

use crate::{ByteArray, Error, IntArray, LongArray, Result, TOKENS, Value, error::bad};
use compound::Compound;
use sequence::Sequence;
use serde::{
    Serialize,
    ser::{self, Impossible},
};

pub(super) struct Serializer;

pub(super) fn value<T: Serialize + ?Sized>(input: &T) -> Result<Value> {
    input
        .serialize(Serializer)?
        .ok_or_else(|| bad("NBT has no null value"))
}

macro_rules! number {
    ($method:ident, $ty:ty, $variant:ident, $cast:ty) => {
        fn $method(self, n: $ty) -> Result<Self::Ok> {
            Ok(Some(Value::$variant(n as $cast)))
        }
    };
}

impl ser::Serializer for Serializer {
    type Ok = Option<Value>;
    type Error = Error;
    type SerializeSeq = Sequence;
    type SerializeTuple = Sequence;
    type SerializeTupleStruct = Sequence;
    type SerializeTupleVariant = Impossible<Self::Ok, Error>;
    type SerializeMap = Compound;
    type SerializeStruct = Compound;
    type SerializeStructVariant = Impossible<Self::Ok, Error>;

    number!(serialize_i8, i8, Byte, i8);
    number!(serialize_i16, i16, Short, i16);
    number!(serialize_i32, i32, Int, i32);
    number!(serialize_i64, i64, Long, i64);
    number!(serialize_u8, u8, Byte, i8);
    number!(serialize_u16, u16, Short, i16);
    number!(serialize_u32, u32, Int, i32);
    number!(serialize_u64, u64, Long, i64);
    number!(serialize_f32, f32, Float, f32);
    number!(serialize_f64, f64, Double, f64);

    fn serialize_bool(self, value: bool) -> Result<Self::Ok> {
        self.serialize_i8(i8::from(value))
    }
    fn serialize_char(self, value: char) -> Result<Self::Ok> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_str(self, value: &str) -> Result<Self::Ok> {
        Ok(Some(Value::String(value.into())))
    }
    fn serialize_bytes(self, bytes: &[u8]) -> Result<Self::Ok> {
        Ok(Some(Value::ByteArray(ByteArray::new(
            bytes.iter().map(|b| *b as i8).collect(),
        ))))
    }
    fn serialize_i128(self, n: i128) -> Result<Self::Ok> {
        self.serialize_u128(n as u128)
    }
    fn serialize_u128(self, n: u128) -> Result<Self::Ok> {
        let ints = n
            .to_be_bytes()
            .chunks_exact(4)
            .map(|b| i32::from_be_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Some(Value::IntArray(IntArray::new(ints))))
    }
    fn serialize_none(self) -> Result<Self::Ok> {
        Ok(None)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Self::Ok> {
        Err(bad("NBT has no unit value"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        name: &'static str,
    ) -> Result<Self::Ok> {
        self.serialize_str(name)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        input: &T,
    ) -> Result<Self::Ok> {
        let Some(index) = TOKENS.iter().position(|token| *token == name) else {
            return input.serialize(self);
        };
        let Value::List(values) = value(input)? else {
            return Err(bad("array needs numeric elements"));
        };
        let array = match index {
            0 => Value::ByteArray(ByteArray::new(
                values
                    .into_iter()
                    .map(|v| match v {
                        Value::Byte(n) => Ok(n),
                        _ => Err(bad("invalid byte array")),
                    })
                    .collect::<Result<_>>()?,
            )),
            1 => Value::IntArray(IntArray::new(
                values
                    .into_iter()
                    .map(|v| match v {
                        Value::Int(n) => Ok(n),
                        _ => Err(bad("invalid int array")),
                    })
                    .collect::<Result<_>>()?,
            )),
            _ => Value::LongArray(LongArray::new(
                values
                    .into_iter()
                    .map(|v| match v {
                        Value::Long(n) => Ok(n),
                        _ => Err(bad("invalid long array")),
                    })
                    .collect::<Result<_>>()?,
            )),
        };
        Ok(Some(array))
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok> {
        Err(bad("NBT enum payloads are unsupported"))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Sequence> {
        Ok(Sequence::new(len.unwrap_or(0)))
    }
    fn serialize_tuple(self, len: usize) -> Result<Sequence> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Sequence> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Err(bad("NBT enum payloads are unsupported"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound> {
        Ok(Compound::default())
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound> {
        self.serialize_map(None)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Err(bad("NBT enum payloads are unsupported"))
    }
}
