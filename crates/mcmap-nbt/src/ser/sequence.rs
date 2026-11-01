use crate::{Error, Result, Value, error::bad};
use serde::{
    Serialize,
    ser::{SerializeSeq, SerializeTuple, SerializeTupleStruct},
};

pub(crate) struct Sequence(Vec<Value>);
impl Sequence {
    pub(super) fn new(len: usize) -> Self {
        Self(Vec::with_capacity(len.min(4096)))
    }
    fn push<T: Serialize + ?Sized>(&mut self, input: &T) -> Result<()> {
        let value = super::value(input)?;
        if self
            .0
            .first()
            .is_some_and(|first| first.tag() != value.tag())
        {
            return Err(bad("NBT list elements must have the same tag"));
        }
        self.0.push(value);
        Ok(())
    }
}

macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl $trait for Sequence {
            type Ok = Option<Value>;
            type Error = Error;
            fn $method<T: Serialize + ?Sized>(&mut self, input: &T) -> Result<()> {
                self.push(input)
            }
            fn end(self) -> Result<Self::Ok> {
                Ok(Some(Value::List(self.0)))
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
