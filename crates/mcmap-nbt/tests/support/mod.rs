use mcmap_nbt::Value;

pub fn canonical(value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode(&mut bytes, value);
    bytes
}

fn text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_be_bytes());
    bytes.extend(value.as_bytes());
}

fn encode(bytes: &mut Vec<u8>, value: &Value) {
    macro_rules! number {
        ($tag:expr, $value:expr) => {{
            bytes.push($tag);
            bytes.extend($value.to_be_bytes());
        }};
    }
    match value {
        Value::Byte(n) => number!(1, n),
        Value::Short(n) => number!(2, n),
        Value::Int(n) => number!(3, n),
        Value::Long(n) => number!(4, n),
        Value::Float(n) => number!(5, n.to_bits()),
        Value::Double(n) => number!(6, n.to_bits()),
        Value::ByteArray(values) => {
            bytes.push(7);
            bytes.extend((values.len() as u64).to_be_bytes());
            bytes.extend(values.iter().map(|n| *n as u8));
        }
        Value::String(s) => {
            bytes.push(8);
            text(bytes, s);
        }
        Value::List(values) => {
            bytes.push(9);
            bytes.extend((values.len() as u64).to_be_bytes());
            for value in values {
                encode(bytes, value);
            }
        }
        Value::Compound(values) => {
            bytes.push(10);
            bytes.extend((values.len() as u64).to_be_bytes());
            let mut sorted = values.iter().collect::<Vec<_>>();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            for (key, value) in sorted {
                text(bytes, key);
                encode(bytes, value);
            }
        }
        Value::IntArray(values) => {
            bytes.push(11);
            bytes.extend((values.len() as u64).to_be_bytes());
            for n in values.iter() {
                bytes.extend(n.to_be_bytes());
            }
        }
        Value::LongArray(values) => {
            bytes.push(12);
            bytes.extend((values.len() as u64).to_be_bytes());
            for n in values.iter() {
                bytes.extend(n.to_be_bytes());
            }
        }
    }
}
