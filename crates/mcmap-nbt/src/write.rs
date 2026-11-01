use crate::{Result, Value, error::bad};
use serde::Serialize;

pub fn to_bytes<T: Serialize + ?Sized>(input: &T) -> Result<Vec<u8>> {
    let value = crate::ser::value(input)?;
    if !matches!(value, Value::Compound(_)) {
        return Err(bad("root is not a compound"));
    }
    let mut out = vec![10, 0, 0];
    payload(&mut out, &value)?;
    Ok(out)
}

fn text(out: &mut Vec<u8>, text: &str) -> Result<()> {
    let bytes = cesu8::to_java_cesu8(text);
    let len = u16::try_from(bytes.len()).map_err(|_| bad("NBT string exceeds 65535 bytes"))?;
    out.extend(len.to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}

fn length(out: &mut Vec<u8>, len: usize) -> Result<()> {
    let len = i32::try_from(len).map_err(|_| bad("NBT collection exceeds i32 length"))?;
    out.extend(len.to_be_bytes());
    Ok(())
}

fn payload(out: &mut Vec<u8>, value: &Value) -> Result<()> {
    match value {
        Value::Byte(n) => out.push(*n as u8),
        Value::Short(n) => out.extend(n.to_be_bytes()),
        Value::Int(n) => out.extend(n.to_be_bytes()),
        Value::Long(n) => out.extend(n.to_be_bytes()),
        Value::Float(n) => out.extend(n.to_be_bytes()),
        Value::Double(n) => out.extend(n.to_be_bytes()),
        Value::String(s) => text(out, s)?,
        Value::ByteArray(values) => {
            length(out, values.len())?;
            out.extend(values.iter().map(|n| *n as u8));
        }
        Value::IntArray(values) => {
            length(out, values.len())?;
            for n in values.iter() {
                out.extend(n.to_be_bytes());
            }
        }
        Value::LongArray(values) => {
            length(out, values.len())?;
            for n in values.iter() {
                out.extend(n.to_be_bytes());
            }
        }
        Value::List(values) => {
            let tag = values.first().map_or(0, Value::tag);
            out.push(tag);
            length(out, values.len())?;
            for value in values {
                if value.tag() != tag {
                    return Err(bad("NBT list elements must have the same tag"));
                }
                payload(out, value)?;
            }
        }
        Value::Compound(values) => {
            for (key, value) in values {
                out.push(value.tag());
                text(out, key)?;
                payload(out, value)?;
            }
            out.push(0);
        }
    }
    Ok(())
}
