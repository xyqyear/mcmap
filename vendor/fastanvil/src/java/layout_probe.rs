use serde::{de::IgnoredAny, Deserialize};

use super::pre18;

#[derive(Deserialize)]
struct Pre18Candidate {
    #[serde(rename = "DataVersion")]
    data_version: i32,
    #[serde(rename = "Level")]
    level: pre18::Level,
    #[serde(rename = "Status")]
    modern_status: Option<IgnoredAny>,
}

pub(super) fn pre18_candidate(data: &[u8]) -> Option<pre18::JavaChunk> {
    if data.first() != Some(&10) {
        return None;
    }
    let name_length = u16::from_be_bytes(data.get(1..3)?.try_into().ok()?) as usize;
    if !data.get(3 + name_length..)?.starts_with(b"\x0a\0\x05Level") {
        return None;
    }
    let candidate: Pre18Candidate = fastnbt::from_bytes(data).ok()?;
    // A missing root Status makes the modern layout impossible, even with a valid nested Level.
    candidate
        .modern_status
        .is_none()
        .then_some(pre18::JavaChunk {
            data_version: candidate.data_version,
            level: candidate.level,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{java::CurrentJavaChunk, JavaChunk};

    fn field(data: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
        data.push(tag);
        data.extend((name.len() as u16).to_be_bytes());
        data.extend(name.as_bytes());
        data.extend(value);
    }

    fn string(value: &str) -> Vec<u8> {
        let mut bytes = (value.len() as u16).to_be_bytes().to_vec();
        bytes.extend(value.as_bytes());
        bytes
    }

    fn root(level_first: bool) -> Vec<u8> {
        let mut level = Vec::new();
        field(&mut level, 3, "xPos", &0i32.to_be_bytes());
        field(&mut level, 3, "zPos", &0i32.to_be_bytes());
        field(&mut level, 8, "Status", &string("full"));
        level.push(0);
        let mut root = vec![10, 0, 0];
        if !level_first {
            field(&mut root, 3, "DataVersion", &2586i32.to_be_bytes());
        }
        field(&mut root, 10, "Level", &level);
        if level_first {
            field(&mut root, 3, "DataVersion", &2586i32.to_be_bytes());
        }
        root
    }

    fn original(data: &[u8]) -> fastnbt::error::Result<JavaChunk> {
        let modern: fastnbt::error::Result<CurrentJavaChunk> = fastnbt::from_bytes(data);
        match modern {
            Ok(chunk) => Ok(JavaChunk::Post18(chunk)),
            Err(modern_error) => match fastnbt::from_bytes::<pre18::JavaChunk>(data) {
                Ok(chunk) => Ok(JavaChunk::Pre18(chunk)),
                Err(pre18_error) => fastnbt::from_bytes::<crate::java::pre13::JavaChunk>(data)
                    .map(JavaChunk::Pre13)
                    .map_err(|pre13_error| serde::de::Error::custom(format!(
                        "Invalid chunk: modern: {modern_error}; pre-1.18: {pre18_error}; pre-1.13: {pre13_error}"
                    ))),
            },
        }
    }

    fn equivalent(data: &[u8]) {
        match (original(data), JavaChunk::from_bytes(data)) {
            (Ok(expected), Ok(actual)) => {
                assert_eq!(format!("{expected:?}"), format!("{actual:?}"))
            }
            (Err(expected), Err(actual)) => assert_eq!(expected.to_string(), actual.to_string()),
            (expected, actual) => {
                panic!("layout mismatch: expected {expected:?}, actual {actual:?}")
            }
        }
    }

    #[test]
    fn pre18_candidate_requires_absent_root_status_and_keeps_modern_priority() {
        let mut old = root(true);
        old.push(0);
        assert!(pre18_candidate(&old).is_some());
        assert!(matches!(
            JavaChunk::from_bytes(&old).unwrap(),
            JavaChunk::Pre18(_)
        ));
        equivalent(&old);
        for status_tag in [3, 8] {
            let mut ambiguous = root(true);
            let value = if status_tag == 3 {
                7i32.to_be_bytes().to_vec()
            } else {
                string("empty")
            };
            field(&mut ambiguous, status_tag, "Status", &value);
            ambiguous.push(0);
            assert!(pre18_candidate(&ambiguous).is_none());
            if status_tag == 8 {
                assert!(matches!(
                    JavaChunk::from_bytes(&ambiguous).unwrap(),
                    JavaChunk::Post18(_)
                ));
            }
            equivalent(&ambiguous);
        }
    }

    #[test]
    fn candidate_preserves_duplicates_wrong_types_and_truncation_errors() {
        let mut cases = Vec::new();
        for level_first in [false, true] {
            let mut valid = root(level_first);
            valid.push(0);
            cases.push(valid);
            for name in ["Status", "DataVersion", "Level"] {
                for tag in [3, 8] {
                    let mut bytes = root(level_first);
                    let value = if tag == 3 {
                        7i32.to_be_bytes().to_vec()
                    } else {
                        string("full")
                    };
                    field(&mut bytes, tag, name, &value);
                    field(&mut bytes, tag, name, &value);
                    bytes.push(0);
                    cases.push(bytes);
                }
            }
        }
        for bytes in cases {
            for end in 0..=bytes.len() {
                equivalent(&bytes[..end]);
            }
        }
    }
}
