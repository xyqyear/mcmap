use super::Serializer;
use crate::{Error, Result, Value, error::bad};
use serde::{
    Serialize,
    ser::{SerializeMap, SerializeStruct},
};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Compound {
    entries: HashMap<String, Value>,
    key: Option<String>,
}

impl Compound {
    fn insert<T: Serialize + ?Sized>(&mut self, key: String, input: &T) -> Result<()> {
        if crate::TOKENS.contains(&key.as_str()) {
            return Err(bad("reserved array key"));
        }
        if let Some(value) = input.serialize(Serializer)? {
            self.entries.insert(key, value);
        }
        Ok(())
    }
}

impl SerializeMap for Compound {
    type Ok = Option<Value>;
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, input: &T) -> Result<()> {
        if self.key.is_some() {
            return Err(bad("missing map value"));
        }
        let Value::String(key) = super::value(input)? else {
            return Err(bad("NBT compound keys must be strings"));
        };
        self.key = Some(key);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, input: &T) -> Result<()> {
        let key = self.key.take().ok_or_else(|| bad("missing map key"))?;
        self.insert(key, input)
    }
    fn end(self) -> Result<Self::Ok> {
        if self.key.is_some() {
            return Err(bad("missing map value"));
        }
        Ok(Some(Value::Compound(self.entries)))
    }
}

impl SerializeStruct for Compound {
    type Ok = Option<Value>;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        input: &T,
    ) -> Result<()> {
        self.insert(key.into(), input)
    }
    fn end(self) -> Result<Self::Ok> {
        SerializeMap::end(self)
    }
}
