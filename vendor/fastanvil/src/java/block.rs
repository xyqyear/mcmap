use std::borrow::Cow;
use std::fmt;

use serde::de::{Error, IgnoredAny, MapAccess, Visitor};
use serde::Deserialize;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct Block {
    pub(crate) name: Cow<'static, str>,
    pub(crate) encoded: Cow<'static, str>,
    pub(crate) archetype: BlockArchetype,
}

#[derive(Debug, PartialEq, Clone)]
pub enum BlockArchetype {
    Normal,
    Airy,
    Watery,
    Snowy,
}

impl Block {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn snowy(&self) -> bool {
        self.archetype == BlockArchetype::Snowy
    }

    /// A string of the format "id|prop1=val1,prop2=val2". The properties are
    /// ordered lexigraphically. This somewhat matches the way Minecraft stores
    /// variants in blockstates, but with the block ID/name prepended.
    pub fn encoded_description(&self) -> &str {
        &self.encoded
    }
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(BlockVisitor)
    }
}

impl Block {
    fn from_state(mut name: Cow<'_, str>, mut properties: Properties<'_>, modern: bool) -> Self {
        if modern {
            if !name.contains(':') {
                name = Cow::Owned(format!("minecraft:{name}"));
            }
            let default = super::default_states_26_3::state(&name);
            if properties.0.is_empty() {
                if let Some(default) = default {
                    return Self {
                        name: Cow::Borrowed(default.name),
                        encoded: Cow::Borrowed(default.encoded),
                        archetype: Self::archetype(default.name, default.snowy),
                    };
                }
                return Self::from_properties(name, std::iter::empty());
            }
            let defaults = default.map(|state| state.properties).unwrap_or(&[]);
            for &(key, value) in defaults {
                if !properties.0.iter().any(|(present, _)| present == key) {
                    properties
                        .0
                        .push((Cow::Borrowed(key), Cow::Borrowed(value)));
                }
            }
        }
        properties.0.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Self::from_properties(
            name,
            properties
                .0
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
        )
    }

    fn from_properties<'a>(
        name: Cow<'_, str>,
        properties: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    ) -> Self {
        let snowy = properties
            .clone()
            .any(|(key, value)| key == "snowy" && value == "true");
        let properties = properties.filter(|(key, _)| *key != "waterlogged" && *key != "powered");
        let capacity = name.len()
            + 1
            + properties
                .clone()
                .map(|(key, value)| key.len() + value.len() + 2)
                .sum::<usize>();
        let mut encoded = String::with_capacity(capacity);
        encoded.push_str(&name);
        encoded.push('|');
        for (index, (key, value)) in properties.enumerate() {
            if index != 0 {
                encoded.push(',');
            }
            encoded.push_str(key);
            encoded.push('=');
            encoded.push_str(value);
        }
        let archetype = Self::archetype(&name, snowy);

        Self {
            name: Cow::Owned(name.into_owned()),
            archetype,
            encoded: Cow::Owned(encoded),
        }
    }

    fn archetype(name: &str, snowy: bool) -> BlockArchetype {
        if snowy {
            BlockArchetype::Snowy
        } else if is_watery(name) {
            BlockArchetype::Watery
        } else if is_airy(name) {
            BlockArchetype::Airy
        } else {
            BlockArchetype::Normal
        }
    }
}

struct Text<'a>(Cow<'a, str>);

impl<'de> Deserialize<'de> for Text<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        mcmap_nbt::borrow::deserialize_cow_str(deserializer).map(Self)
    }
}

#[derive(Default)]
struct Properties<'a>(Vec<(Cow<'a, str>, Cow<'a, str>)>);

impl<'de> Deserialize<'de> for Properties<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PropertiesVisitor;
        impl<'de> Visitor<'de> for PropertiesVisitor {
            type Value = Properties<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("block state properties containing strings")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut properties = Properties::default();
                while let Some((Text(key), Text(value))) = map.next_entry::<Text, Text>()? {
                    if let Some((_, existing)) = properties.0.iter_mut().find(|(k, _)| *k == key) {
                        *existing = value;
                    } else {
                        properties.0.push((key, value));
                    }
                }
                Ok(properties)
            }
        }
        deserializer.deserialize_map(PropertiesVisitor)
    }
}

#[derive(Deserialize)]
#[serde(field_identifier)]
enum Field {
    Name,
    Properties,
    #[serde(rename = "id")]
    Id,
    #[serde(rename = "properties")]
    ModernProperties,
    #[serde(rename = "")]
    Wrapped,
    #[serde(other)]
    Unknown,
}

struct BlockVisitor;

impl<'de> Visitor<'de> for BlockVisitor {
    type Value = Block;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a block palette entry containing a string or compound")
    }

    fn visit_str<E: Error>(self, name: &str) -> Result<Block, E> {
        Ok(Block::from_state(
            Cow::Borrowed(name),
            Properties::default(),
            true,
        ))
    }

    fn visit_string<E: Error>(self, name: String) -> Result<Block, E> {
        Ok(Block::from_state(
            Cow::Owned(name),
            Properties::default(),
            true,
        ))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Block, M::Error> {
        let mut name = None;
        let mut id = None;
        let mut wrapped = None;
        let mut properties = Properties::default();
        let mut modern_properties = Properties::default();
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Name => name = Some(map.next_value::<Text>()?.0),
                Field::Id => id = Some(map.next_value::<Text>()?.0),
                Field::Wrapped => wrapped = Some(map.next_value::<Text>()?.0),
                Field::Properties => properties = map.next_value()?,
                Field::ModernProperties => modern_properties = map.next_value()?,
                Field::Unknown => {
                    let _ = map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if let Some(name) = name {
            return Ok(Block::from_state(name, properties, false));
        }
        let name = id
            .or(wrapped)
            .ok_or_else(|| M::Error::custom("Block palette entry must contain a string id"))?;
        Ok(Block::from_state(name, modern_properties, true))
    }
}

/// Blocks that are considered as if they are water when determining colour.
fn is_watery(block: &str) -> bool {
    matches!(
        block,
        "minecraft:water"
            | "minecraft:bubble_column"
            | "minecraft:kelp"
            | "minecraft:kelp_plant"
            | "minecraft:sea_grass"
            | "minecraft:tall_seagrass"
    )
}

/// Blocks that are considered as if they are air when determining colour.
fn is_airy(block: &str) -> bool {
    matches!(block, "minecraft:air" | "minecraft:cave_air")
}
