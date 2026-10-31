use fastnbt::{nbt, LongArray};

use super::{Chunk, ChunkData, HeightMode};

fn heights(values: [u16; 256]) -> LongArray {
    let mut longs = vec![0_i64; 37];
    for (index, value) in values.into_iter().enumerate() {
        longs[index / 7] |= i64::from(value) << ((index % 7) * 9);
    }
    LongArray::new(longs)
}

#[test]
fn bulk_trusted_heights_keep_column_order() {
    let values = std::array::from_fn(|index| index as u16);
    let bytes = fastnbt::to_bytes(&nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "Heightmaps": {"MOTION_BLOCKING": heights(values)},
        "sections": [{"Y": 0_i8, "block_states": {"palette": ["minecraft:stone"]}}]
    }))
    .unwrap();
    let chunk = ChunkData::from_bytes(&bytes).unwrap();
    assert_eq!(
        chunk.surface_heights(HeightMode::Trust),
        values.map(|height| height as isize)
    );
    for (index, expected) in values.into_iter().enumerate() {
        assert_eq!(
            chunk.surface_height(index % 16, index / 16, HeightMode::Trust),
            expected as isize
        );
    }
}

#[test]
fn bulk_heights_preserve_negative_terminator_correction() {
    let bytes = fastnbt::to_bytes(&nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "Heightmaps": {"MOTION_BLOCKING": heights([16; 256])},
        "sections": [
            {"Y": -5_i8, "block_states": {"palette": ["minecraft:air"]}},
            {"Y": -4_i8, "block_states": {"palette": ["minecraft:stone"]}}
        ]
    }))
    .unwrap();
    let chunk = ChunkData::from_bytes(&bytes).unwrap();
    assert_eq!(chunk.y_range(), -64..-48);
    assert_eq!(chunk.surface_heights(HeightMode::Trust), [-48; 256]);
    for index in 0..256 {
        assert_eq!(
            chunk.surface_height(index % 16, index / 16, HeightMode::Trust),
            -48
        );
    }

    let scalar_chunk = ChunkData::from_bytes(&bytes).unwrap();
    let expected = std::array::from_fn(|index| {
        scalar_chunk.surface_height(index % 16, index / 16, HeightMode::Calculate)
    });
    assert_eq!(chunk.surface_heights(HeightMode::Calculate), expected);
}

#[test]
fn nonstandard_heightmap_packing_still_forces_calculation() {
    let bytes = fastnbt::to_bytes(&nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "Heightmaps": {"MOTION_BLOCKING": LongArray::new(vec![0; 29])},
        "sections": [{"Y": 0_i8, "block_states": {"palette": ["minecraft:stone"]}}]
    }))
    .unwrap();
    for mode in [HeightMode::Trust, HeightMode::Calculate] {
        let chunk = ChunkData::from_bytes(&bytes).unwrap();
        assert_eq!(chunk.surface_heights(mode), [16; 256]);
        for index in 0..256 {
            assert_eq!(chunk.surface_height(index % 16, index / 16, mode), 16);
        }
    }
}

#[test]
fn water_depth_preserves_explicit_lower_bound_with_corrected_height_range() {
    let bytes = fastnbt::to_bytes(&nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "Heightmaps": {"MOTION_BLOCKING": heights([16; 256])},
        "sections": [
            {"Y": -5_i8, "block_states": {"palette": ["minecraft:air"]}},
            {"Y": -4_i8, "block_states": {"palette": ["minecraft:water"]}}
        ]
    }))
    .unwrap();
    let chunk = ChunkData::from_bytes(&bytes).unwrap();
    assert_eq!(chunk.y_range(), -64..-48);
    assert_eq!(chunk.water_depth(0, -49, 0, -50), 2);
    assert_eq!(chunk.water_depth(0, -49, 0, -64), 16);
    assert_eq!(chunk.water_depth(0, -49, 0, -80), 17);
}
