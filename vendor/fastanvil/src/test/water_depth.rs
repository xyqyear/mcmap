use mcmap_nbt::{nbt, LongArray, Value};

use crate::{BlockArchetype, Chunk, JavaChunk};

fn decode(sections: Value) -> JavaChunk {
    let bytes = mcmap_nbt::to_bytes(&nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "sections": sections
    }))
    .unwrap();
    JavaChunk::from_bytes(&bytes).unwrap()
}

fn scalar_depth(chunk: &impl Chunk, x: usize, mut y: isize, z: usize, y_min: isize) -> isize {
    let mut depth = 1;
    while y > y_min {
        let Some(block) = chunk.block(x, y, z) else {
            return depth;
        };
        if block.archetype != BlockArchetype::Watery {
            return depth;
        }
        depth += 1;
        y -= 1;
    }
    depth
}

fn assert_matches_scalar(chunk: &impl Chunk) {
    for x in 0..16 {
        for z in 0..16 {
            for y in [-35, -17, -16, -1, 0, 8, 15, 16] {
                for y_min in [-64, -32, -5, 0, 15, 32] {
                    assert_eq!(
                        chunk.water_depth(x, y, z, y_min),
                        scalar_depth(chunk, x, y, z, y_min),
                        "x={x}, y={y}, z={z}, y_min={y_min}"
                    );
                }
            }
        }
    }
}

#[test]
fn uniform_sections_keep_lower_bounds_and_missing_section_stops() {
    let water = decode(nbt!([
        {"Y": -2_i8, "block_states": {"palette": ["minecraft:water"]}},
        {"Y": -1_i8, "block_states": {"palette": ["minecraft:kelp"]}},
        {"Y": 0_i8, "block_states": {"palette": ["minecraft:water"]}}
    ]));
    assert_eq!(water.water_depth(0, 15, 0, -32), 48);
    assert_eq!(water.water_depth(0, 15, 0, -64), 49);
    assert_eq!(water.water_depth(0, 15, 0, 3), 13);
    assert_eq!(water.water_depth(0, 15, 0, 15), 1);
    assert_matches_scalar(&water);

    let gap = decode(nbt!([
        {"Y": -2_i8, "block_states": {"palette": ["minecraft:water"]}},
        {"Y": 0_i8, "block_states": {"palette": ["minecraft:water"]}}
    ]));
    assert_eq!(gap.water_depth(0, 15, 0, -64), 17);
    assert_matches_scalar(&gap);
}

#[test]
fn packed_and_missing_states_keep_air_fallbacks() {
    let mut indices = vec![0_i64; 256];
    indices[224] = 15;
    let packed = decode(nbt!([{"Y": 0_i8, "block_states": {
        "palette": ["minecraft:water", "minecraft:kelp"],
        "data": LongArray::new(indices)
    }}]));
    assert_eq!(packed.water_depth(0, 15, 0, 0), 2);
    assert_matches_scalar(&packed);

    let mut indices = vec![0_i64; 256];
    indices[224] = 1;
    let singleton = decode(nbt!([{"Y": 0_i8, "block_states": {
        "palette": ["minecraft:water"],
        "data": LongArray::new(indices)
    }}]));
    assert_eq!(singleton.water_depth(0, 15, 0, 0), 2);
    assert_matches_scalar(&singleton);

    for states in [
        nbt!({"palette": ["minecraft:water", "minecraft:kelp"]}),
        nbt!({"palette": []}),
    ] {
        let empty = decode(nbt!([{"Y": 0_i8, "block_states": states}]));
        assert_eq!(empty.water_depth(0, 15, 0, 0), 1);
        assert_matches_scalar(&empty);
    }
}

#[test]
fn scalar_default_remains_compatible_with_older_chunk_layouts() {
    for bytes in [
        include_bytes!("../../resources/1.12.chunk").as_slice(),
        include_bytes!("../../resources/1.17.1.chunk").as_slice(),
    ] {
        let chunk = JavaChunk::from_bytes(bytes).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                for y in [0, 32, 64, 96, 255] {
                    assert_eq!(
                        chunk.water_depth(x, y, z, 0),
                        scalar_depth(&chunk, x, y, z, 0)
                    );
                }
            }
        }
    }
}
