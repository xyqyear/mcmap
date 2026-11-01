mod de;
mod ser;

use crate::{ByteArray, IntArray, LongArray};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(ByteArray),
    String(String),
    List(Vec<Value>),
    Compound(HashMap<String, Value>),
    IntArray(IntArray),
    LongArray(LongArray),
}

impl Value {
    pub(crate) fn tag(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Byte(n) => Some(*n as i64),
            Self::Short(n) => Some(*n as i64),
            Self::Int(n) => Some(*n as i64),
            Self::Long(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(n) => Some(*n as f64),
            Self::Double(n) => Some(*n),
            _ => self.as_i64().map(|n| n as f64),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
}

macro_rules! value_from {
    ($($ty:ty => $variant:ident),* $(,)?) => {$ (
        impl From<$ty> for Value {
            fn from(value: $ty) -> Self { Self::$variant(value) }
        }
    )*};
}
value_from! {
    i8 => Byte, i16 => Short, i32 => Int, i64 => Long, f32 => Float, f64 => Double,
    String => String, ByteArray => ByteArray, IntArray => IntArray, LongArray => LongArray,
    HashMap<String, Value> => Compound,
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(value: Vec<T>) -> Self {
        Self::List(value.into_iter().map(Into::into).collect())
    }
}
