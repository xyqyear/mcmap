// Rendering chunks to overhead maps

use super::block::BlockArchetype;
use super::chunk::{Chunk, ChunkData, HeightMode};
use super::region::{CCoord, RCoord, RegionLoader};

pub type Rgba = [u8; 4];
pub use fastanvil::RenderedPalette;

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
        match chunk {
            super::chunk::ChunkData::Post13(post13) => {
                return self.render_post13(post13, north);
            }
            super::chunk::ChunkData::Pre13(_) => {
                // Continue with our custom Pre-1.13 rendering
            }
        }

        let mut data = [[0, 0, 0, 0]; 16 * 16];

        let y_range = chunk.y_range();

        for z in 0..16 {
            for x in 0..16 {
                let air_height = chunk.surface_height(x, z, self.height_mode);
                let block_height = (air_height - 1).max(y_range.start);

                let colour = self.drill_for_colour(x, block_height, z, chunk, y_range.start);

                let north_air_height = match z {
                    0 => north
                        .map(|c| c.surface_height(x, 15, self.height_mode))
                        .unwrap_or(block_height),
                    z => chunk.surface_height(x, z - 1, self.height_mode),
                };
                let colour = top_shade_colour(colour, air_height, north_air_height);

                data[z * 16 + x] = colour;
            }
        }

        data
    }

    fn render_post13(&self, chunk: &super::chunk::Post13Chunk, north: Option<&ChunkData>) -> [Rgba; 16 * 16] {
        // Use fastanvil's renderer for Post-1.13 chunks
        let fa_mode = match self.height_mode {
            HeightMode::Trust => fastanvil::HeightMode::Trust,
            HeightMode::Calculate => fastanvil::HeightMode::Calculate,
        };

        let fa_renderer = fastanvil::TopShadeRenderer::new(self.palette, fa_mode);

        // Get north chunk as fastanvil JavaChunk if available
        let north_fa = north.and_then(|n| {
            if let super::chunk::ChunkData::Post13(p13) = n {
                Some(p13.inner())
            } else {
                None
            }
        });

        fa_renderer.render(chunk.inner(), north_fa)
    }

    /// Look up color for Pre-1.13 block from palette
    fn pick_color_for_pre13_block(&self, block: &super::block::Block) -> Rgba {
        // Look up by full block name in the loaded palette
        if let Some(&color) = self.palette.blockstates.get(block.name) {
            return color;
        }

        // Fallback: try without "minecraft:" prefix for legacy names
        let name_without_prefix = block.name.strip_prefix("minecraft:").unwrap_or(block.name);
        let legacy_name = format!("minecraft:{}", name_without_prefix);
        if let Some(&color) = self.palette.blockstates.get(&legacy_name) {
            return color;
        }

        // Default for unknown blocks: magenta to indicate missing palette entry
        [255, 0, 255, 255]
    }

    fn drill_for_colour(
        &self,
        x: usize,
        y_start: isize,
        z: usize,
        chunk: &ChunkData,
        y_min: isize,
    ) -> Rgba {
        let mut y = y_start;
        let mut colour = [0, 0, 0, 0];

        while colour[3] != 255 && y >= y_min {
            let current_block = chunk.block(x, y, z);

            if let Some(current_block) = current_block {
                match current_block.archetype {
                    BlockArchetype::Airy => {
                        y -= 1;
                    }
                    BlockArchetype::Watery => {
                        let mut block_colour = self.pick_color_for_pre13_block(current_block);
                        let water_depth = water_depth(x, y, z, chunk, y_min);
                        let alpha = water_depth_to_alpha(water_depth);

                        block_colour[3] = alpha;

                        colour = a_over_b_colour(colour, block_colour);
                        y -= water_depth;
                    }
                    _ => {
                        let block_colour = self.pick_color_for_pre13_block(current_block);
                        colour = a_over_b_colour(colour, block_colour);
                        y -= 1;
                    }
                }
            } else {
                return colour;
            }
        }

        colour
    }
}

fn water_depth_to_alpha(water_depth: isize) -> u8 {
    (180 + 2 * water_depth).min(250) as u8
}

fn water_depth(x: usize, mut y: isize, z: usize, chunk: &ChunkData, y_min: isize) -> isize {
    let mut depth = 1;
    while y > y_min {
        let block = match chunk.block(x, y, z) {
            Some(b) => b,
            None => return depth,
        };

        if block.archetype != BlockArchetype::Watery {
            return depth;
        }

        y -= 1;
        depth += 1;
    }
    depth
}

fn a_over_b_colour(a: Rgba, b: Rgba) -> Rgba {
    let a_a = a[3] as f32 / 255.0;
    let b_a = b[3] as f32 / 255.0;

    let out_a = a_a + b_a * (1.0 - a_a);

    if out_a == 0.0 {
        return [0, 0, 0, 0];
    }

    let out_r = (a[0] as f32 * a_a + b[0] as f32 * b_a * (1.0 - a_a)) / out_a;
    let out_g = (a[1] as f32 * a_a + b[1] as f32 * b_a * (1.0 - a_a)) / out_a;
    let out_b = (a[2] as f32 * a_a + b[2] as f32 * b_a * (1.0 - a_a)) / out_a;

    [
        out_r as u8,
        out_g as u8,
        out_b as u8,
        (out_a * 255.0) as u8,
    ]
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
