// 1.13+ top-shade renderer. Implements `RenderEngine` by delegating to
// fastanvil's renderer, with our own `Palette` impl that overrides two cases
// fastanvil gets wrong for modded servers:
//
//   1. Mod-defined air variants (`compactmachines:machine_void_air`,
//      `botania:fake_air`, …) aren't in the palette, so fastanvil's `pick`
//      returns its magenta `missing_colour` placeholder. The pixel comes out
//      solid magenta. Treat any block whose name ends in `_air` (or `:air`
//      for the namespace-only form) as transparent — those blocks are
//      conceptually nothing.
//   2. fastanvil hard-codes `minecraft:air` to opaque black `[0,0,0,255]`
//      ("Occurs a lot for the end, as layer 0 will be air in the void.").
//      That assumption was specific to the End; for the void / mod dims it
//      produces a pure-black render where transparent would be more truthful.
//
// Both cases collapse to the same rule: any block whose name ends in `_air`
// or `:air` is transparent. Everything else defers to fastanvil.

use std::collections::HashMap;

use super::chunk::{ChunkData, HeightMode};
use crate::anvil::pipeline::{RenderEngine, Rgba};

/// Modern palette: name → RGBA. Implements `fastanvil::Palette` directly so
/// the per-chunk render path doesn't need a wrapper. The `blockstates` map
/// stays public for direct lookup in the analyze command.
pub struct RenderedPalette {
    pub blockstates: HashMap<String, Rgba>,
    fa_palette: fastanvil::RenderedPalette,
}

impl std::fmt::Debug for RenderedPalette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedPalette")
            .field("blockstates_len", &self.blockstates.len())
            .finish()
    }
}

impl RenderedPalette {
    pub fn new(blockstates: HashMap<String, Rgba>) -> Self {
        use image::RgbaImage;

        // fastanvil wants biome colormaps for grass/foliage tinting. We don't
        // apply biome tints at render time, so feed it flat dummy maps.
        let grass = RgbaImage::from_pixel(256, 256, image::Rgba([100, 150, 50, 255]));
        let foliage = RgbaImage::from_pixel(256, 256, image::Rgba([50, 100, 30, 255]));

        let fa_palette = fastanvil::RenderedPalette {
            blockstates: blockstates.clone(),
            grass,
            foliage,
        };

        Self {
            blockstates,
            fa_palette,
        }
    }
}

impl fastanvil::Palette for RenderedPalette {
    fn pick(&self, block: &fastanvil::Block, biome: Option<fastanvil::biome::Biome>) -> Rgba {
        // We deliberately don't broaden this to substrings like `void` —
        // `theabyss:black_void` is a real solid block, not a hole.
        let name = block.name();
        if name.ends_with("_air") || name.ends_with(":air") {
            return [0, 0, 0, 0];
        }
        self.fa_palette.pick(block, biome)
    }
}

/// 1.13+ top-shade renderer — plugs into the shared region pipeline.
pub struct TopShadeRenderer<'a> {
    palette: &'a RenderedPalette,
    height_mode: HeightMode,
}

impl<'a> TopShadeRenderer<'a> {
    pub fn new(palette: &'a RenderedPalette, mode: HeightMode) -> Self {
        Self {
            palette,
            height_mode: mode,
        }
    }
}

impl<'a> RenderEngine for TopShadeRenderer<'a> {
    type Chunk = ChunkData;

    fn decode(&self, bytes: &[u8]) -> Result<Option<Self::Chunk>, String> {
        ChunkData::from_bytes(bytes).map(Some)
    }

    fn render_chunk(&self, chunk: &ChunkData, north: Option<&ChunkData>) -> [Rgba; 16 * 16] {
        let fa_renderer = fastanvil::TopShadeRenderer::new(self.palette, self.height_mode);
        fa_renderer.render(chunk, north)
    }
}
