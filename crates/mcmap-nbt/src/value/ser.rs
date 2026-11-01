use super::Value;
use serde::{Serialize, Serializer};

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Byte(n) => serializer.serialize_i8(*n),
            Self::Short(n) => serializer.serialize_i16(*n),
            Self::Int(n) => serializer.serialize_i32(*n),
            Self::Long(n) => serializer.serialize_i64(*n),
            Self::Float(n) => serializer.serialize_f32(*n),
            Self::Double(n) => serializer.serialize_f64(*n),
            Self::String(s) => serializer.serialize_str(s),
            Self::List(values) => values.serialize(serializer),
            Self::Compound(values) => values.serialize(serializer),
            Self::ByteArray(values) => values.serialize(serializer),
            Self::IntArray(values) => values.serialize(serializer),
            Self::LongArray(values) => values.serialize(serializer),
        }
    }
}
