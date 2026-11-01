use serde::de::{self, Visitor};
use std::{borrow::Cow, fmt};

pub fn deserialize_cow_str<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Cow<'de, str>, D::Error> {
    struct Text;
    impl<'de> Visitor<'de> for Text {
        type Value = Cow<'de, str>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string")
        }
        fn visit_borrowed_str<E: de::Error>(self, text: &'de str) -> Result<Self::Value, E> {
            Ok(Cow::Borrowed(text))
        }
        fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
            Ok(Cow::Owned(text.to_owned()))
        }
        fn visit_string<E: de::Error>(self, text: String) -> Result<Self::Value, E> {
            Ok(Cow::Owned(text))
        }
    }
    deserializer.deserialize_str(Text)
}
