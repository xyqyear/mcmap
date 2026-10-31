use fastnbt::{nbt, LongArray};
use serde::Deserialize;

use super::{bits_per_block, Pre18Blockstates};
use crate::{Chunk, JavaChunk, PackedBits};

fn states(words: &[i64]) -> Pre18Blockstates {
    #[derive(Deserialize)]
    struct Data {
        states: Pre18Blockstates,
    }
    let bytes = fastnbt::to_bytes(&nbt!({"states":LongArray::new(words.to_vec())})).unwrap();
    fastnbt::from_bytes::<Data>(&bytes).unwrap().states
}

fn words(len: usize) -> Vec<i64> {
    (0..len)
        .map(|i| {
            (i as u64)
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add(0xfedcba9876543210) as i64
        })
        .collect()
}

fn compare(words: &[i64], palette: usize) {
    let mut expected = [0u16; 4096];
    PackedBits(LongArray::new(words.to_vec()))
        .unpack_blockstates(bits_per_block(palette), &mut expected);
    let actual = states(words);
    for (i, value) in expected.iter().enumerate() {
        assert_eq!(
            actual.state(i & 15, i >> 8, (i >> 4) & 15, palette),
            *value as usize,
            "length={},palette={palette},index={i}",
            words.len()
        );
    }
    assert_eq!(
        actual.iter_indices(palette).collect::<Vec<_>>(),
        expected.map(usize::from)
    );
}

#[test]
fn padded_lengths_keep_width_selection_and_every_column() {
    for len in [256, 342, 410, 456, 512, 586] {
        for palette in [0, 1, 2, 16, 17, 33, 129, 513, 4096] {
            compare(&words(len), palette);
        }
    }
}

#[test]
fn continuous_and_nonstandard_lengths_keep_original_unpacking() {
    for (len, palette) in [
        (320, 17),
        (321, 17),
        (384, 33),
        (448, 65),
        (640, 513),
        (704, 1025),
        (832, 4097),
        (1024, 32769),
        (1152, 131073),
    ] {
        compare(&words(len), palette);
    }
    let actual = states(&words(320));
    let first = actual.state(0, 0, 0, 17);
    assert_eq!(actual.state(0, 0, 0, 65535), first);
    assert_eq!(actual.iter_indices(65535).next(), Some(first));
    let actual = states(&words(320));
    let first = actual.iter_indices(17).next();
    assert_eq!(Some(actual.state(0, 0, 0, 65535)), first);
}

#[test]
fn short_data_and_out_of_range_access_still_panic() {
    for len in [0, 1, 255, 319] {
        for index in [0, 4095] {
            let actual = states(&words(len));
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| actual.state(
                    index & 15,
                    index >> 8,
                    (index >> 4) & 15,
                    17
                )))
                .is_err()
            );
        }
    }
    for len in [256, 342, 320] {
        let actual = states(&words(len));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || actual.state(0, 16, 0, 17)
        ))
        .is_err());
    }
}

#[test]
fn singleton_and_invalid_palette_indices_keep_chunk_block_behavior() {
    for (data, expected) in [
        (Some(vec![0i64; 256]), Some("minecraft:water")),
        (Some(vec![-1i64; 256]), None),
        (None, Some("minecraft:air")),
    ] {
        let mut section = nbt!({"Y":0i8,"Palette":[{"Name":"minecraft:water"}]});
        if let Some(words) = data {
            let fastnbt::Value::Compound(map) = &mut section else {
                unreachable!()
            };
            map.insert("BlockStates".into(), LongArray::new(words).into());
        }
        let bytes=fastnbt::to_bytes(&nbt!({"DataVersion":2586,"Level":{"xPos":0,"zPos":0,"Status":"full","Sections":[section]}})).unwrap();
        let chunk = JavaChunk::from_bytes(&bytes).unwrap();
        assert_eq!(chunk.block(0, 0, 0).map(|block| block.name()), expected);
        assert_eq!(chunk.block(15, 15, 15).map(|block| block.name()), expected);
    }
}
