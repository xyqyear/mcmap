use std::collections::HashMap;
use std::sync::LazyLock;

use fastanvil::{CurrentJavaChunk, JavaChunk};
use fastnbt::Value;

static DEFAULT_PROPERTIES: LazyLock<HashMap<String, HashMap<String, String>>> =
    LazyLock::new(|| {
        serde_json::from_str(include_str!("default_states_26_3.json"))
            .expect("embedded Minecraft default states must be valid")
    });

pub fn decode(data: &[u8]) -> Result<JavaChunk, String> {
    match fastnbt::from_bytes::<CurrentJavaChunk>(data) {
        Ok(chunk) => Ok(JavaChunk::Post18(chunk)),
        Err(modern_error) => {
            let mut root: Value = fastnbt::from_bytes(data).map_err(|error| error.to_string())?;
            let Value::Compound(fields) = &mut root else {
                return Err("Chunk root must be a compound".into());
            };
            if !fields.contains_key("sections") {
                return JavaChunk::from_bytes(data).map_err(|error| error.to_string());
            }
            let Some(Value::List(sections)) = fields.get_mut("sections") else {
                return Err(modern_error.to_string());
            };
            for section in sections {
                let Value::Compound(section) = section else {
                    return Err("Chunk section must be a compound".into());
                };
                let Some(Value::Compound(states)) = section.get_mut("block_states") else {
                    continue;
                };
                let Some(Value::List(palette)) = states.get_mut("palette") else {
                    continue;
                };
                for state in palette {
                    normalize_state(state)?;
                }
            }
            fastnbt::from_value::<CurrentJavaChunk>(&root)
                .map(JavaChunk::Post18)
                .map_err(|error| format!("Invalid modern chunk: {error}"))
        }
    }
}

fn normalize_state(state: &mut Value) -> Result<(), String> {
    let (name, properties) = match state {
        Value::String(name) => (name.clone(), None),
        Value::Compound(fields) => {
            if fields.contains_key("Name") {
                return Ok(());
            }
            let name = fields.get("id").or_else(|| fields.get(""));
            let Some(Value::String(name)) = name else {
                return Err("Block palette entry must contain a string id".into());
            };
            let properties = match fields.get("properties") {
                None => None,
                Some(Value::Compound(properties)) => Some(properties.clone()),
                Some(_) => return Err("Block state properties must be a compound".into()),
            };
            (name.clone(), properties)
        }
        _ => return Err("Block palette entry must be a string or compound".into()),
    };
    let name = if name.contains(':') {
        name
    } else {
        format!("minecraft:{name}")
    };
    let mut resolved: HashMap<String, Value> = DEFAULT_PROPERTIES
        .get(&name)
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect();
    if let Some(properties) = properties {
        if properties
            .values()
            .any(|value| !matches!(value, Value::String(_)))
        {
            return Err("Block state property values must be strings".into());
        }
        resolved.extend(properties);
    }
    *state = Value::Compound(HashMap::from([
        ("Name".into(), Value::String(name)),
        ("Properties".into(), Value::Compound(resolved)),
    ]));
    Ok(())
}
