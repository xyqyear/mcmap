use std::ops::Range;

use crate::{biome::Biome, Block, Chunk, HeightMode, JavaChunk};

#[test]
fn bulk_heights_match_scalar_queries_for_each_chunk_layout() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("../../resources/1.12.chunk"),
        include_bytes!("../../resources/1.17.1.chunk"),
        include_bytes!("../../resources/21w44a-test1.nbt"),
        include_bytes!("../../resources/forge-1.20.1.nbt"),
    ];
    for (fixture_index, bytes) in fixtures.iter().enumerate() {
        for mode in [HeightMode::Trust, HeightMode::Calculate] {
            let scalar_chunk = JavaChunk::from_bytes(bytes).unwrap();
            let bulk_chunk = JavaChunk::from_bytes(bytes).unwrap();
            let expected = std::array::from_fn(|index| {
                scalar_chunk.surface_height(index % 16, index / 16, mode)
            });
            assert_eq!(
                bulk_chunk.surface_heights(mode),
                expected,
                "fixture {fixture_index}, {mode:?}"
            );
            assert_eq!(bulk_chunk.surface_heights(mode), expected);
        }
    }
}

struct ExternalChunk {
    heights: [isize; 256],
    block: Block,
}

impl ExternalChunk {
    fn flat(height: isize) -> Self {
        Self {
            heights: [height; 256],
            block: serde_json::from_str(r#"{"Name":"minecraft:stone"}"#).unwrap(),
        }
    }
}

impl Chunk for ExternalChunk {
    fn status(&self) -> String {
        "full".into()
    }

    fn surface_height(&self, x: usize, z: usize, _: HeightMode) -> isize {
        self.heights[z * 16 + x]
    }

    fn biome(&self, _: usize, _: isize, _: usize) -> Option<Biome> {
        None
    }

    fn block(&self, _: usize, y: isize, _: usize) -> Option<&Block> {
        self.y_range().contains(&y).then_some(&self.block)
    }

    fn y_range(&self) -> Range<isize> {
        -64..320
    }
}

#[test]
fn external_chunks_receive_row_major_bulk_heights_by_default() {
    let mut chunk = ExternalChunk::flat(0);
    chunk.heights = std::array::from_fn(|index| index as isize - 64);
    assert_eq!(chunk.surface_heights(HeightMode::Trust), chunk.heights);
    assert_eq!(chunk.surface_heights(HeightMode::Calculate), chunk.heights);
}

#[cfg(feature = "render")]
#[test]
fn bulk_height_rendering_preserves_northern_and_internal_row_shading() {
    use crate::{Palette, Rgba, TopShadeRenderer};

    struct SolidPalette;
    impl Palette for SolidPalette {
        fn pick(&self, _: &Block, _: Option<Biome>) -> Rgba {
            [200, 100, 60, 255]
        }
    }

    let renderer = TopShadeRenderer::new(&SolidPalette, HeightMode::Trust);
    let chunk = ExternalChunk::flat(10);
    let north = ExternalChunk::flat(12);
    let with_north = renderer.render(&chunk, Some(&north));
    assert_eq!(&with_north[..16], &[[141, 70, 42, 255]; 16]);
    assert!(with_north[16..]
        .iter()
        .all(|pixel| *pixel == [172, 86, 51, 255]));

    let without_north = renderer.render(&chunk, None);
    assert_eq!(&without_north[..16], &[[200, 100, 60, 255]; 16]);
    assert_eq!(&without_north[16..], &with_north[16..]);
}
