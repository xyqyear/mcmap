use mcmap_nbt::{ByteArray, IntArray, LongArray, Value, from_bytes, nbt, to_bytes};
use serde::{Deserialize, Serialize};

fn named(tag: u8, name: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    bytes.extend((name.len() as u16).to_be_bytes());
    bytes.extend(name.as_bytes());
    bytes.extend(payload);
    bytes
}

fn root(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![10, 0, 0];
    for field in fields {
        bytes.extend(field);
    }
    bytes.push(0);
    bytes
}

#[test]
fn independent_wire_fixture_preserves_tags_and_endianness() {
    let bytes = root(&[
        named(1, "b", &[255]),
        named(2, "s", &(-123i16).to_be_bytes()),
        named(3, "i", &(123456i32).to_be_bytes()),
        named(4, "l", &i64::MIN.to_be_bytes()),
        named(5, "f", &1.25f32.to_be_bytes()),
        named(6, "d", &(-2.5f64).to_be_bytes()),
        named(7, "bytes", &[0, 0, 0, 2, 0, 255]),
        named(8, "text", &[0, 3, b'a', b'b', b'c']),
        named(9, "list", &[2, 0, 0, 0, 2, 0, 1, 255, 255]),
        named(10, "map", &[0]),
        named(11, "ints", &[0, 0, 0, 1, 255, 255, 255, 255]),
        named(12, "longs", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7]),
    ]);
    let expected = nbt!({
        "b": -1i8, "s": -123i16, "i": 123456, "l": i64::MIN,
        "f": 1.25f32, "d": -2.5f64, "bytes": [B; 0, -1], "text": "abc",
        "list": [1i16,-1i16], "map": {}, "ints": [I; -1], "longs": [L; 7],
    });
    assert_eq!(from_bytes::<Value>(&bytes).unwrap(), expected);
    assert_eq!(
        from_bytes::<Value>(&to_bytes(&expected).unwrap()).unwrap(),
        expected
    );
    for end in 0..bytes.len() {
        assert!(
            from_bytes::<Value>(&bytes[..end]).is_err(),
            "truncation {end}"
        );
    }
}

#[test]
fn array_types_own_their_contents_and_do_not_become_lists() {
    #[derive(Deserialize, Serialize, Debug, PartialEq)]
    struct Arrays {
        b: ByteArray,
        i: IntArray,
        l: LongArray,
    }
    let expected = Arrays {
        b: ByteArray::new(vec![-128, 127]),
        i: IntArray::new(vec![i32::MIN, i32::MAX]),
        l: LongArray::new(vec![i64::MIN, i64::MAX]),
    };
    let mut bytes = to_bytes(&expected).unwrap();
    let actual: Arrays = from_bytes(&bytes).unwrap();
    bytes.fill(0);
    drop(bytes);
    assert_eq!(actual, expected);
    #[derive(Deserialize)]
    struct List {
        #[allow(dead_code)]
        b: Vec<i8>,
    }
    assert!(from_bytes::<List>(&to_bytes(&expected).unwrap()).is_err());
    assert!(
        from_bytes::<Arrays>(&to_bytes(&nbt!({"b": [1i8], "i": [I;], "l": [L;]})).unwrap())
            .is_err()
    );
}

#[test]
fn modified_utf8_strings_and_names_round_trip() {
    let expected = nbt!({"nul\0中文🦀": "\0中文🦀"});
    let bytes = to_bytes(&expected).unwrap();
    assert!(bytes.windows(2).any(|b| b == [0xc0, 0x80]));
    assert!(bytes.windows(3).any(|b| b == [0xed, 0xa0, 0xbe]));
    assert_eq!(from_bytes::<Value>(&bytes).unwrap(), expected);
    assert!(from_bytes::<Value>(&root(&[named(8, "v", &[0, 1, 0xff])])).is_err());
}

#[test]
fn floating_point_bits_are_not_normalized() {
    #[derive(Deserialize)]
    struct Float {
        v: f32,
    }
    #[derive(Deserialize)]
    struct Double {
        v: f64,
    }
    for bits in [
        0,
        0x80000000,
        1,
        0x007fffff,
        0x7f800000,
        0xff800000,
        0x7fc01234,
        0x7f801234u32,
    ] {
        assert_eq!(
            from_bytes::<Float>(&root(&[named(5, "v", &bits.to_be_bytes())]))
                .unwrap()
                .v
                .to_bits(),
            bits
        );
    }
    for bits in [
        0,
        0x8000000000000000,
        1,
        0x7ff0000000000000,
        0x7ff0000000001234u64,
    ] {
        assert_eq!(
            from_bytes::<Double>(&root(&[named(6, "v", &bits.to_be_bytes())]))
                .unwrap()
                .v
                .to_bits(),
            bits
        );
    }
}

#[test]
fn numeric_coercions_are_checked_and_uuid_int_arrays_preserve_bits() {
    #[derive(Deserialize)]
    struct Small {
        v: i8,
    }
    #[derive(Deserialize)]
    struct Truth {
        v: bool,
    }
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Uuid {
        v: i128,
    }
    assert_eq!(
        from_bytes::<Small>(&root(&[named(3, "v", &127i32.to_be_bytes())]))
            .unwrap()
            .v,
        127
    );
    assert!(from_bytes::<Small>(&root(&[named(3, "v", &128i32.to_be_bytes())])).is_err());
    assert!(
        from_bytes::<Truth>(&root(&[named(4, "v", &(-1i64).to_be_bytes())]))
            .unwrap()
            .v
    );
    for v in [i128::MIN, -1, 0, i128::MAX] {
        assert_eq!(
            from_bytes::<Uuid>(&to_bytes(&Uuid { v }).unwrap()).unwrap(),
            Uuid { v }
        );
    }
}

#[test]
fn duplicate_struct_fields_fail_but_compound_entries_keep_last_value() {
    #[derive(Deserialize)]
    struct Record {
        #[allow(dead_code)]
        v: i32,
    }
    let bytes = root(&[
        named(3, "v", &1i32.to_be_bytes()),
        named(3, "v", &2i32.to_be_bytes()),
    ]);
    assert!(from_bytes::<Record>(&bytes).is_err());
    assert_eq!(from_bytes::<Value>(&bytes).unwrap(), nbt!({"v":2}));
    assert!(from_bytes::<Record>(&root(&[])).is_err());
}

#[test]
fn unknown_deep_compounds_and_lists_are_skipped_without_native_stack_growth() {
    #[derive(Deserialize)]
    struct Record {
        answer: i32,
    }
    for depth in [0, 1, 63, 64, 65, 128, 1024, 10_000] {
        for list in [false, true] {
            let mut payload = Vec::new();
            for _ in 0..depth {
                if list {
                    payload.extend([9, 0, 0, 10, 0, 0, 0, 1]);
                } else {
                    payload.extend([10, 0, 0]);
                }
            }
            payload.resize(payload.len() + depth + 1, 0);
            let bytes = root(&[
                named(10, "ignored", &payload),
                named(3, "answer", &42i32.to_be_bytes()),
            ]);
            assert_eq!(from_bytes::<Record>(&bytes).unwrap().answer, 42);
            assert!(from_bytes::<Record>(&bytes[..bytes.len() - 2]).is_err());
        }
    }
}

#[test]
fn malformed_lengths_types_and_reserved_keys_are_rejected() {
    for tag in [7, 11, 12] {
        for len in [-1i32, i32::MAX] {
            assert!(from_bytes::<Value>(&root(&[named(tag, "v", &len.to_be_bytes())])).is_err());
        }
    }
    for payload in [
        &[0, 0, 0, 0, 1][..],
        &[13, 0, 0, 0, 0][..],
        &[1, 255, 255, 255, 255][..],
    ] {
        assert!(from_bytes::<Value>(&root(&[named(9, "v", payload)])).is_err());
    }
    assert!(from_bytes::<Value>(&root(&[named(13, "v", &[])])).is_err());
    assert!(
        from_bytes::<Value>(&root(&[named(
            3,
            "__fastnbt_long_array",
            &0i32.to_be_bytes()
        )]))
        .is_err()
    );
}

#[test]
fn serializer_rejects_invalid_wire_shapes_and_omits_none_fields() {
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Record {
        name: String,
        missing: Option<i32>,
    }
    let value = Record {
        name: "test".into(),
        missing: None,
    };
    let bytes = to_bytes(&value).unwrap();
    assert_eq!(from_bytes::<Value>(&bytes).unwrap(), nbt!({"name":"test"}));
    assert_eq!(from_bytes::<Record>(&bytes).unwrap(), value);
    assert!(to_bytes(&nbt!({"list":[1i8,2i32]})).is_err());
    assert!(to_bytes(&nbt!({"text":"x".repeat(65536)})).is_err());
    assert!(to_bytes(&Value::Int(1)).is_err());
}

#[test]
fn excessively_nested_materialized_values_return_an_error() {
    let mut bytes = vec![10, 0, 0];
    for _ in 0..10_000 {
        bytes.extend([10, 0, 0]);
    }
    bytes.resize(bytes.len() + 10_001, 0);
    assert!(from_bytes::<Value>(&bytes).is_err());
}
