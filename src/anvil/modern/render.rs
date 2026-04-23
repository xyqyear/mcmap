// 1.13+ top-shade renderer. Implements `RenderEngine` by delegating to
// fastanvil.

use std::collections::HashMap;

use super::chunk::{ChunkData, HeightMode};
use crate::anvil::pipeline::{RenderEngine, Rgba};

/// Modern palette: name → RGBA. Kept public for direct lookup in the analyze
/// command; the per-chunk rendering path builds a `fastanvil::RenderedPalette`
/// once up front (clone of the same map) so we don't allocate one per chunk.
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

    fn fastanvil_palette(&self) -> &fastanvil::RenderedPalette {
        &self.fa_palette
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

    fn render_chunk(
        &self,
        chunk: &ChunkData,
        north: Option<&ChunkData>,
    ) -> [Rgba; 16 * 16] {
        let fa_renderer =
            fastanvil::TopShadeRenderer::new(self.palette.fastanvil_palette(), self.height_mode);
        let north_fa = north.map(|n| n.inner());
        fa_renderer.render(chunk.inner(), north_fa)
    }
}
