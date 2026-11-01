use fastanvil::{Block, Palette, biome::Biome};
use image::RgbaImage;
use serde_json::json;

use super::{BlockColors, RenderedPalette};

#[test]
fn flat_tints_and_fast_lookup_match_the_previous_palette() {
    let colors: BlockColors = serde_json::from_value(json!({
        "minecraft:stone": [31, 47, 128, 255],
        "minecraft:snow_block": [241, 242, 243, 255],
        "example:glass": [97, 81, 203, 64],
        "example:glass|variant=blue": [12, 34, 56, 78],
        "example:not_air_block": [100, 110, 120, 255]
    }))
    .unwrap();
    let previous = fastanvil::RenderedPalette {
        blockstates: colors
            .iter()
            .map(|(name, color)| (name.clone(), *color))
            .collect(),
        grass: RgbaImage::from_pixel(256, 256, image::Rgba([100, 150, 50, 255])),
        foliage: RgbaImage::from_pixel(256, 256, image::Rgba([50, 100, 30, 255])),
    };
    let current = RenderedPalette::new(colors);
    for name in [
        "minecraft:stone",
        "minecraft:grass",
        "minecraft:tall_grass",
        "minecraft:fern",
        "minecraft:grass_block",
        "minecraft:water",
        "minecraft:bubble_column",
        "minecraft:oak_leaves",
        "minecraft:birch_leaves",
        "minecraft:spruce_leaves",
        "minecraft:mangrove_leaves",
        "minecraft:kelp",
        "minecraft:seagrass",
        "minecraft:snow",
        "minecraft:air",
        "minecraft:cave_air",
        "example:void_air",
        "example:air",
        "example:not_air_block",
        "example:glass",
        "example:unmapped",
    ] {
        for properties in [
            json!({}),
            json!({"snowy":"true"}),
            json!({"variant":"blue"}),
        ] {
            let block: Block =
                serde_json::from_value(json!({"Name":name,"Properties":properties})).unwrap();
            for biome in [
                None,
                Some(Biome::Plains),
                Some(Biome::Swamp),
                Some(Biome::WarmOcean),
                Some(Biome::FrozenOcean),
            ] {
                let expected = if name.ends_with("_air") || name.ends_with(":air") {
                    [0, 0, 0, 0]
                } else {
                    previous.pick(&block, biome)
                };
                assert_eq!(current.pick(&block, biome), expected, "{name} {biome:?}");
            }
        }
    }
}

#[test]
fn separate_palettes_do_not_share_resolved_colors() {
    let block: Block = serde_json::from_value(json!({"Name":"example:solid"})).unwrap();
    let first =
        RenderedPalette::new(serde_json::from_value(json!({"example:solid":[1,2,3,4]})).unwrap());
    let second = RenderedPalette::new(
        serde_json::from_value(json!({"example:solid":[201,202,203,204]})).unwrap(),
    );
    assert_eq!(first.pick(&block, None), [1, 2, 3, 4]);
    assert_eq!(second.pick(&block, None), [201, 202, 203, 204]);
    assert_eq!(first.pick(&block, None), [1, 2, 3, 4]);
}

fn boundary_chunk(status: &str, section_y: Option<i8>) -> super::ChunkData {
    use mcmap_nbt::{LongArray, nbt};
    let mut value = nbt!({
        "DataVersion": 5023,
        "Status": status,
    });
    let mcmap_nbt::Value::Compound(root) = &mut value else {
        unreachable!()
    };
    if let Some(y) = section_y {
        let mut heights = vec![0_i64; 37];
        for index in 0..256 {
            heights[index / 7] |= 16_i64 << ((index % 7) * 9);
        }
        root.insert(
            "Heightmaps".into(),
            nbt!({"MOTION_BLOCKING": LongArray::new(heights)}),
        );
        root.insert(
            "sections".into(),
            nbt!([{"Y": y, "block_states": {"palette": ["minecraft:stone"]}}]),
        );
    } else {
        root.insert(
            "Heightmaps".into(),
            nbt!({"MOTION_BLOCKING": LongArray::new(vec![0;37])}),
        );
    }
    super::ChunkData::from_bytes(&mcmap_nbt::to_bytes(&value).unwrap()).unwrap()
}

#[test]
fn north_boundaries_keep_skipped_and_seed_chunks_lazy() {
    use crate::anvil::pipeline::RenderEngine;
    let palette = RenderedPalette::new(
        serde_json::from_value(json!({"minecraft:stone":[200,100,60,255]})).unwrap(),
    );
    let renderer = super::TopShadeRenderer::new(&palette, super::HeightMode::Trust);
    let seed = renderer.boundary(boundary_chunk("minecraft:full", None));
    let (pixels, skipped) =
        renderer.render_chunk(boundary_chunk("minecraft:noise", None), Some(&seed));
    assert_eq!(pixels, [[0; 4]; 256]);
    assert!(matches!(skipped, super::NorthBoundary::Deferred(_)));
    let requested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.render_chunk(boundary_chunk("minecraft:full", Some(0)), Some(&seed))
    }));
    assert!(requested.is_err());
}

#[test]
fn cached_southern_heights_match_seed_and_skipped_northern_chunks() {
    use crate::anvil::pipeline::RenderEngine;
    let palette = RenderedPalette::new(
        serde_json::from_value(json!({"minecraft:stone":[200,100,60,255]})).unwrap(),
    );
    let renderer = super::TopShadeRenderer::new(&palette, super::HeightMode::Trust);
    let seed = renderer.boundary(boundary_chunk("minecraft:full", Some(1)));
    let (_, cached) = renderer.render_chunk(boundary_chunk("minecraft:full", Some(1)), None);
    assert!(matches!(&cached, super::NorthBoundary::Heights(heights) if *heights == [32; 16]));
    let (_, skipped) = renderer.render_chunk(boundary_chunk("minecraft:noise", Some(1)), None);
    let expected = renderer
        .render_chunk(boundary_chunk("minecraft:full", Some(0)), Some(&seed))
        .0;
    assert_eq!(&expected[..16], &[[141, 70, 42, 255]; 16]);
    for north in [&cached, &skipped] {
        assert_eq!(
            renderer
                .render_chunk(boundary_chunk("minecraft:full", Some(0)), Some(north))
                .0,
            expected
        );
    }
}
