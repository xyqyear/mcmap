// Rendering chunks to overhead maps

use super::chunk::{Chunk, ChunkData, HeightMode};
use super::region::{CCoord, RCoord, RegionLoader};
use std::collections::HashMap;

pub type Rgba = [u8; 4];

/// Simplified palette that maps block names to RGBA colors
pub struct RenderedPalette {
    /// Map from block name (with optional state) to RGBA color.
    /// Kept for callers that query the palette directly (e.g. analyze command).
    pub blockstates: HashMap<String, Rgba>,
    /// Fastanvil-compatible palette used for Post-1.13 chunk rendering.
    /// Built once when the palette is loaded so chunk rendering does not
    /// allocate a fresh one per chunk.
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

        // Dummy 256x256 colormaps — fastanvil expects them but we don't use
        // biome-based coloring, so a single flat color per map is enough.
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

    pub fn fastanvil_palette(&self) -> &fastanvil::RenderedPalette {
        &self.fa_palette
    }
}

/// Map of a rendered region
pub struct RegionMap {
    pub x: RCoord,
    pub z: RCoord,
    data: Vec<Rgba>,
}

impl RegionMap {
    pub fn new(x: RCoord, z: RCoord, fill: Rgba) -> Self {
        let len = 32 * 16 * 32 * 16; // 32 chunks * 16 blocks * 32 chunks * 16 blocks
        Self {
            x,
            z,
            data: vec![fill; len],
        }
    }

    pub fn chunk(&self, x: CCoord, z: CCoord) -> &[Rgba] {
        let len = 16 * 16;
        let begin = (z.0 * 32 + x.0) as usize * len;
        &self.data[begin..begin + len]
    }

    fn chunk_mut(&mut self, x: CCoord, z: CCoord) -> &mut [Rgba] {
        let len = 16 * 16;
        let begin = (z.0 * 32 + x.0) as usize * len;
        &mut self.data[begin..begin + len]
    }
}

/// Top-down renderer with shading
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

    pub fn render(&self, chunk: &ChunkData, north: Option<&ChunkData>) -> [Rgba; 16 * 16] {
        // For Post-1.13 chunks, use fastanvil's rendering directly
        if let super::chunk::ChunkData::Post13(post13) = chunk {
            return self.render_post13(post13, north);
        }

        // For Pre-1.13 chunks: simple gray heightmap with shading
        let mut data = [[0, 0, 0, 0]; 16 * 16];

        for z in 0..16 {
            for x in 0..16 {
                let air_height = chunk.surface_height(x, z, self.height_mode);

                // Use fixed gray color for all Pre-1.13 blocks
                let colour = [128, 128, 128, 255];

                let north_air_height = match z {
                    0 => north
                        .map(|c| c.surface_height(x, 15, self.height_mode))
                        .unwrap_or(air_height),
                    z => chunk.surface_height(x, z - 1, self.height_mode),
                };
                let colour = top_shade_colour(colour, air_height, north_air_height);

                data[z * 16 + x] = colour;
            }
        }

        data
    }

    fn render_post13(
        &self,
        chunk: &super::chunk::Post13Chunk,
        north: Option<&ChunkData>,
    ) -> [Rgba; 16 * 16] {
        let fa_mode = match self.height_mode {
            HeightMode::Trust => fastanvil::HeightMode::Trust,
            HeightMode::Calculate => fastanvil::HeightMode::Calculate,
        };

        let fa_renderer =
            fastanvil::TopShadeRenderer::new(self.palette.fastanvil_palette(), fa_mode);

        let north_fa = north.and_then(|n| {
            if let super::chunk::ChunkData::Post13(p13) = n {
                Some(p13.inner())
            } else {
                None
            }
        });

        fa_renderer.render(chunk.inner(), north_fa)
    }
}

fn top_shade_colour(colour: Rgba, height: isize, north_height: isize) -> Rgba {
    let diff = height - north_height;

    let shade = match diff.cmp(&0) {
        std::cmp::Ordering::Greater => 0.8,
        std::cmp::Ordering::Less => 1.2,
        std::cmp::Ordering::Equal => 1.0,
    };

    [
        (colour[0] as f32 * shade).min(255.0) as u8,
        (colour[1] as f32 * shade).min(255.0) as u8,
        (colour[2] as f32 * shade).min(255.0) as u8,
        colour[3],
    ]
}

/// Render a region to a map
pub fn render_region(
    x: RCoord,
    z: RCoord,
    loader: &dyn RegionLoader,
    renderer: TopShadeRenderer,
) -> Result<Option<RegionMap>, String> {
    let mut map = RegionMap::new(x, z, [0u8; 4]);

    let mut region = match loader.region(x, z)? {
        Some(r) => r,
        None => return Ok(None),
    };

    let mut cache: [Option<ChunkData>; 32] = Default::default();

    // Cache the last row of chunks from the above region for top-shading
    if let Ok(Some(mut r)) = loader.region(x, RCoord(z.0 - 1)) {
        for (x, entry) in cache.iter_mut().enumerate() {
            *entry = r
                .read_chunk(x, 31)
                .ok()
                .flatten()
                .and_then(|b| ChunkData::from_bytes(&b).ok())
        }
    }

    for z in 0usize..32 {
        for (x, cache) in cache.iter_mut().enumerate() {
            let data = map.chunk_mut(CCoord(x as isize), CCoord(z as isize));

            let chunk_data = region
                .read_chunk(x, z)
                .map_err(|e| format!("Failed to read chunk: {}", e))?;

            let chunk_data = match chunk_data {
                Some(data) => data,
                None => continue,
            };

            let chunk = ChunkData::from_bytes(&chunk_data)?;

            let north = cache.as_ref();

            let rendered = renderer.render(&chunk, north);
            data.copy_from_slice(&rendered);

            *cache = Some(chunk);
        }
    }

    Ok(Some(map))
}
