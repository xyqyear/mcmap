// Rendering chunks to overhead maps

use super::block::BlockArchetype;
use super::chunk::{Chunk, ChunkData, HeightMode};
use super::region::{CCoord, RCoord, RegionLoader};
use std::collections::HashMap;

pub type Rgba = [u8; 4];

/// Simplified palette that maps block names to RGBA colors
#[derive(Debug, Clone)]
pub struct RenderedPalette {
    /// Map from block name (with optional state) to RGBA color
    pub blockstates: HashMap<String, Rgba>,
    /// Map from combined block ID (block_id << 4 + metadata) to block name for Pre-1.13
    pub idmap: HashMap<u16, String>,
}

impl RenderedPalette {
    /// Create a fastanvil-compatible palette for Post-1.13 rendering
    /// This creates dummy grass/foliage colormaps since we don't use biome colors
    pub fn to_fastanvil_palette(&self) -> fastanvil::RenderedPalette {
        use image::RgbaImage;

        // Create dummy 256x256 colormap images (standard size for Minecraft colormaps)
        let grass_map = RgbaImage::from_pixel(256, 256, image::Rgba([100, 150, 50, 255]));
        let foliage_map = RgbaImage::from_pixel(256, 256, image::Rgba([50, 100, 30, 255]));

        fastanvil::RenderedPalette {
            blockstates: self.blockstates.clone(),
            grass: grass_map,
            foliage: foliage_map,
        }
    }

    /// Get block name from Pre-1.13 block ID and metadata
    pub fn get_block_name(&self, block_id: u16, metadata: u8) -> Option<&str> {
        let combined_id = (block_id << 4) | (metadata as u16);
        self.idmap.get(&combined_id).map(|s| s.as_str())
    }

    /// Get color for a Pre-1.13 block
    /// Now uses O(1) lookup since palette.json has base colors for all blocks
    pub fn get_color_for_pre13(&self, block_id: u16, metadata: u8) -> Rgba {
        if let Some(block_name) = self.get_block_name(block_id, metadata) {
            // Direct O(1) lookup
            if let Some(&color) = self.blockstates.get(block_name) {
                return color;
            }
        }

        // Default for unknown blocks: magenta to indicate missing palette entry
        [255, 0, 255, 255]
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

    fn render_post13(
        &self,
        chunk: &super::chunk::Post13Chunk,
        north: Option<&ChunkData>,
    ) -> [Rgba; 16 * 16] {
        // Use fastanvil's renderer for Post-1.13 chunks
        let fa_mode = match self.height_mode {
            HeightMode::Trust => fastanvil::HeightMode::Trust,
            HeightMode::Calculate => fastanvil::HeightMode::Calculate,
        };

        // Convert our palette to fastanvil format
        let fa_palette = self.palette.to_fastanvil_palette();
        let fa_renderer = fastanvil::TopShadeRenderer::new(&fa_palette, fa_mode);

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
        // For Pre-1.13 chunks, use the raw block ID lookup
        if let super::chunk::ChunkData::Pre13(pre13) = chunk {
            return self.drill_for_colour_pre13(x, y_start, z, pre13, y_min);
        }

        // For Post-1.13, use the old method (though this shouldn't be called for Post-1.13)
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

    fn drill_for_colour_pre13(
        &self,
        x: usize,
        y_start: isize,
        z: usize,
        chunk: &super::chunk::Pre13Chunk,
        y_min: isize,
    ) -> Rgba {
        let mut y = y_start;
        let mut colour = [0, 0, 0, 0];

        while colour[3] != 255 && y >= y_min {
            // Get raw block ID and metadata
            if let Some((block_id, metadata)) = chunk.raw_block(x, y, z) {
                // Check if it's air (block_id 0)
                if block_id == 0 {
                    y -= 1;
                    continue;
                }

                // Check if it's water (block_id 8 or 9)
                if block_id == 8 || block_id == 9 {
                    let mut block_colour = self.palette.get_color_for_pre13(block_id, metadata);
                    let water_depth = water_depth_pre13(x, y, z, chunk, y_min);
                    let alpha = water_depth_to_alpha(water_depth);

                    block_colour[3] = alpha;

                    colour = a_over_b_colour(colour, block_colour);
                    y -= water_depth;
                } else {
                    // Solid block
                    let block_colour = self.palette.get_color_for_pre13(block_id, metadata);
                    colour = a_over_b_colour(colour, block_colour);
                    y -= 1;
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

fn water_depth_pre13(
    x: usize,
    mut y: isize,
    z: usize,
    chunk: &super::chunk::Pre13Chunk,
    y_min: isize,
) -> isize {
    let mut depth = 1;
    while y > y_min {
        let (block_id, _) = match chunk.raw_block(x, y, z) {
            Some(b) => b,
            None => return depth,
        };

        // Check if it's water (block_id 8 or 9)
        if block_id != 8 && block_id != 9 {
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

    [out_r as u8, out_g as u8, out_b as u8, (out_a * 255.0) as u8]
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
