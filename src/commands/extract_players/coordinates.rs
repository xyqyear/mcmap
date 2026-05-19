use fastnbt::Value;
use flate2::read::GzDecoder;
use std::io::Read;
use std::path::Path;

use super::output::{Position, SkipReason};

#[derive(Debug)]
pub struct ExtractedCoordinates {
    pub data_version: Option<i32>,
    pub dim: String,
    pub pos: Position,
}

#[derive(Debug)]
pub struct ExtractError {
    pub reason: SkipReason,
    pub message: Option<String>,
}

impl ExtractError {
    fn new(reason: SkipReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: Some(message.into()),
        }
    }

    fn reason(reason: SkipReason) -> Self {
        Self {
            reason,
            message: None,
        }
    }
}

pub fn extract(path: &Path) -> Result<ExtractedCoordinates, ExtractError> {
    let bytes = std::fs::read(path)
        .map_err(|e| ExtractError::new(SkipReason::ParseError, e.to_string()))?;
    extract_from_bytes(&bytes)
}

pub fn extract_from_bytes(bytes: &[u8]) -> Result<ExtractedCoordinates, ExtractError> {
    let nbt = read_nbt(bytes)?;
    let Value::Compound(root) = nbt else {
        return Err(ExtractError::new(
            SkipReason::ParseError,
            "root NBT tag is not a compound",
        ));
    };
    let pos_value = root
        .get("Pos")
        .ok_or_else(|| ExtractError::reason(SkipReason::MissingPos))?;
    let pos = extract_pos(pos_value)?;
    let dim_value = root
        .get("Dimension")
        .ok_or_else(|| ExtractError::reason(SkipReason::MissingDimension))?;
    let dim = extract_dimension(dim_value)?;
    let data_version = match root.get("DataVersion") {
        Some(Value::Int(v)) => Some(*v),
        _ => None,
    };
    Ok(ExtractedCoordinates {
        data_version,
        dim,
        pos,
    })
}

fn read_nbt(bytes: &[u8]) -> Result<Value, ExtractError> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut dec = GzDecoder::new(bytes);
        let mut buf = Vec::with_capacity(bytes.len() * 4);
        dec.read_to_end(&mut buf)
            .map_err(|e| ExtractError::new(SkipReason::ParseError, e.to_string()))?;
        fastnbt::from_bytes(&buf)
            .map_err(|e| ExtractError::new(SkipReason::ParseError, e.to_string()))
    } else {
        fastnbt::from_bytes(bytes)
            .map_err(|e| ExtractError::new(SkipReason::ParseError, e.to_string()))
    }
}

fn extract_pos(value: &Value) -> Result<Position, ExtractError> {
    let Value::List(list) = value else {
        return Err(ExtractError::reason(SkipReason::InvalidPos));
    };
    if list.len() != 3 {
        return Err(ExtractError::reason(SkipReason::InvalidPos));
    }
    let x = numeric(&list[0]).ok_or_else(|| ExtractError::reason(SkipReason::InvalidPos))?;
    let y = numeric(&list[1]).ok_or_else(|| ExtractError::reason(SkipReason::InvalidPos))?;
    let z = numeric(&list[2]).ok_or_else(|| ExtractError::reason(SkipReason::InvalidPos))?;
    Ok(Position { x, y, z })
}

fn extract_dimension(value: &Value) -> Result<String, ExtractError> {
    match value {
        Value::String(s) if !s.is_empty() => Ok(s.clone()),
        Value::Int(v) => Ok(v.to_string()),
        Value::Byte(v) => Ok(v.to_string()),
        Value::Short(v) => Ok(v.to_string()),
        Value::Long(v) => i32::try_from(*v)
            .map(|v| v.to_string())
            .map_err(|_| ExtractError::reason(SkipReason::InvalidDimension)),
        _ => Err(ExtractError::reason(SkipReason::InvalidDimension)),
    }
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Double(v) => Some(*v),
        Value::Float(v) => Some(*v as f64),
        Value::Int(v) => Some(*v as f64),
        Value::Long(v) => Some(*v as f64),
        Value::Short(v) => Some(*v as f64),
        Value::Byte(v) => Some(*v as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;

    #[test]
    fn extracts_modern_dimension_string() {
        let pos = Value::List(vec![
            Value::Double(1.25),
            Value::Double(64.0),
            Value::Double(-2.5),
        ]);
        assert_eq!(
            extract_pos(&pos).unwrap(),
            Position {
                x: 1.25,
                y: 64.0,
                z: -2.5
            }
        );
        assert_eq!(
            extract_dimension(&Value::String("minecraft:overworld".into())).unwrap(),
            "minecraft:overworld"
        );
    }

    #[test]
    fn extracts_legacy_dimension_int() {
        assert_eq!(extract_dimension(&Value::Int(7)).unwrap(), "7");
    }

    #[test]
    fn rejects_bad_position() {
        let pos = Value::List(vec![Value::Double(1.0), Value::Double(2.0)]);
        let err = extract_pos(&pos).unwrap_err();
        assert_eq!(err.reason, SkipReason::InvalidPos);
    }
}
