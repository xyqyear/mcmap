// Chunk data structures for 1.7.10 (Pre-1.13) format
// Supports both standard Blocks/Data and non-standard Blocks16/Data16

use fastnbt::{ByteArray, IntArray};
use serde::Deserialize;

use super::block::{AIR, Block, BlockArchetype, STONE};

#[derive(Debug, Clone, Copy)]
pub enum HeightMode {
    Trust,     // Use heightmap data from chunk
    Calculate, // Calculate from block data
}

/// Trait for chunk types
pub trait Chunk {
    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize;
    fn block(&self, x: usize, y: isize, z: usize) -> Option<&'static Block>;
    fn y_range(&self) -> std::ops::Range<isize>;
}

/// Chunk wrapper that can handle both Pre-1.13 and Post-1.13 formats
#[derive(Debug)]
pub enum ChunkData {
    Pre13(Pre13Chunk),
    Post13(Post13Chunk),
}

/// Pre-1.13 chunk (1.7.10 - 1.12.2)
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Pre13Chunk {
    pub level: Level,
}

/// Post-1.13 chunk (1.13+) - using fastanvil's JavaChunk
#[derive(Debug)]
pub struct Post13Chunk {
    inner: fastanvil::JavaChunk,
}

impl Post13Chunk {
    pub fn inner(&self) -> &fastanvil::JavaChunk {
        &self.inner
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Level {
    #[serde(rename = "xPos")]
    #[allow(dead_code)]
    pub x_pos: i32,
    #[serde(rename = "zPos")]
    #[allow(dead_code)]
    pub z_pos: i32,
    pub sections: Option<Vec<Section>>,
    pub height_map: Option<IntArray>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Section {
    #[serde(rename = "Y")]
    pub y: i8,

    // Standard 1.7.10 format
    #[serde(default)]
    pub blocks: Option<ByteArray>,
    #[serde(default)]
    pub data: Option<ByteArray>,
    #[serde(default)]
    pub add: Option<ByteArray>,

    // Non-standard format (some modded servers)
    #[serde(default)]
    pub blocks16: Option<ByteArray>,
    #[serde(default)]
    pub data16: Option<ByteArray>,
}

impl Section {
    /// Get raw block_id and data_value for a block position
    /// Returns (block_id, data_value)
    pub fn raw_block(&self, x: usize, sec_y: usize, z: usize) -> Option<(u16, u8)> {
        let idx: usize = (sec_y << 8) + (z << 4) + x;

        // Try standard Blocks format first
        if let Some(blocks) = &self.blocks {
            let mut block_id = blocks[idx] as u8 as u16;

            // Add extra bits from Add field if present
            if let Some(add) = &self.add {
                let mut add_id = add[idx / 2] as u8;
                if idx % 2 == 0 {
                    add_id &= 0x0F;
                } else {
                    add_id = (add_id & 0xF0) >> 4;
                }
                block_id += (add_id as u16) << 8;
            }

            let data_value = if let Some(data) = &self.data {
                let d = data[idx / 2] as u8;
                if idx % 2 == 0 {
                    d & 0x0F
                } else {
                    (d & 0xF0) >> 4
                }
            } else {
                0
            };

            return Some((block_id, data_value));
        }

        // Try Blocks16 format (non-standard)
        if let Some(blocks16) = &self.blocks16 {
            // Blocks16 is 8192 bytes (double size, likely 16 bits per block)
            let idx16 = idx * 2;
            if idx16 + 1 < blocks16.len() {
                let block_id =
                    u16::from_le_bytes([blocks16[idx16] as u8, blocks16[idx16 + 1] as u8]);

                let data_value = if let Some(data16) = &self.data16 {
                    let idx16_data = idx * 2;
                    if idx16_data < data16.len() {
                        data16[idx16_data] as u8 & 0x0F
                    } else {
                        0
                    }
                } else {
                    0
                };

                return Some((block_id & 0xFFF, data_value));
            }
        }

        None
    }

    fn block(&self, x: usize, sec_y: usize, z: usize) -> &'static Block {
        if let Some((block_id, data_value)) = self.raw_block(x, sec_y, z) {
            return simple_block_lookup(block_id, data_value);
        }
        &AIR
    }
}

/// Helper struct to read DataVersion
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ChunkVersionCheck {
    #[serde(rename = "DataVersion")]
    data_version: Option<i32>,
    level: Option<serde::de::IgnoredAny>,
}

impl ChunkData {
    /// Parse chunk data from bytes, auto-detecting format based on DataVersion
    ///
    /// Version detection based on DataVersion field:
    /// - < 1344: 1.7.10-1.12 (Pre-1.13, has "Level" tag)
    /// - 1344-2555: 1.15 (Post-1.13, has "Level" tag)
    /// - 2556-2824: 1.16-1.17 (Post-1.13, has "Level" tag)
    /// - >= 2825: 1.18+ (Post-1.13, NO "Level" tag!)
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        // First, try to read DataVersion to determine chunk format
        let version_info: Result<ChunkVersionCheck, _> = fastnbt::from_bytes(data);

        match version_info {
            Ok(info) => {
                let data_version = info.data_version.unwrap_or(-1);
                let has_level = info.level.is_some();

                if data_version < 1344 || (data_version < 0 && has_level) {
                    // Pre-1.13 format: 1.7.10-1.12
                    // Has "Level" tag with Blocks/Data arrays
                    match fastnbt::from_bytes::<Pre13Chunk>(data) {
                        Ok(chunk) => Ok(ChunkData::Pre13(chunk)),
                        Err(e) => Err(format!(
                            "Failed to parse as Pre-1.13 (DataVersion {}): {}",
                            data_version, e
                        )),
                    }
                } else {
                    // Post-1.13 format: 1.13+
                    // Uses fastanvil's JavaChunk (handles palette-based storage)
                    match fastanvil::JavaChunk::from_bytes(data) {
                        Ok(inner) => Ok(ChunkData::Post13(Post13Chunk { inner })),
                        Err(e) => Err(format!(
                            "Failed to parse as Post-1.13 (DataVersion {}): {}",
                            data_version, e
                        )),
                    }
                }
            }
            Err(_) => {
                // If we can't read version info, try both formats
                // Try Post-1.13 first (more common in recent versions)
                if let Ok(inner) = fastanvil::JavaChunk::from_bytes(data) {
                    return Ok(ChunkData::Post13(Post13Chunk { inner }));
                }

                // Fall back to Pre-1.13
                match fastnbt::from_bytes::<Pre13Chunk>(data) {
                    Ok(chunk) => Ok(ChunkData::Pre13(chunk)),
                    Err(e) => Err(format!("Failed to parse chunk (unknown version): {}", e)),
                }
            }
        }
    }
}

impl Chunk for Pre13Chunk {
    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        match mode {
            HeightMode::Trust => {
                if let Some(ref height_map) = self.level.height_map {
                    // Height map is stored as int array
                    // Each value is 32 bits but we need to extract the actual height
                    let idx = z * 16 + x;
                    if idx < height_map.len() {
                        let raw = height_map[idx];
                        // Extract height from packed value
                        // In 1.7.10, heightmap values can be large, need to handle properly
                        return (raw & 0xFF) as isize;
                    }
                }
                // Fallthrough to calculate
            }
            HeightMode::Calculate => {}
        }

        // Calculate from blocks
        let y_range = self.y_range();
        for y in (y_range.start..y_range.end).rev() {
            if let Some(block) = self.block(x, y, z) {
                if block.archetype != BlockArchetype::Airy {
                    return y + 1;
                }
            }
        }
        0
    }

    fn block(&self, x: usize, y: isize, z: usize) -> Option<&'static Block> {
        let sections = self.level.sections.as_ref()?;
        let section_y = (y >> 4) as i8;

        let section = sections.iter().find(|s| s.y == section_y)?;
        let sec_y = (y & 0xF) as usize;

        Some(section.block(x, sec_y, z))
    }

    fn y_range(&self) -> std::ops::Range<isize> {
        if let Some(ref sections) = self.level.sections {
            if sections.is_empty() {
                return 0..0;
            }
            let min_y = sections.iter().map(|s| s.y).min().unwrap_or(0) as isize * 16;
            let max_y = (sections.iter().map(|s| s.y).max().unwrap_or(0) as isize + 1) * 16;
            min_y..max_y
        } else {
            0..0
        }
    }
}

/// Simplified block lookup - only distinguishes air from non-air blocks
fn simple_block_lookup(block_id: u16, _data_value: u8) -> &'static Block {
    match block_id {
        0 => &AIR,
        _ => &STONE, // All non-air blocks treated as solid
    }
}

// Post-1.13 Chunk implementation
impl Chunk for Post13Chunk {
    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        let fa_mode = match mode {
            HeightMode::Trust => fastanvil::HeightMode::Trust,
            HeightMode::Calculate => fastanvil::HeightMode::Calculate,
        };
        fastanvil::Chunk::surface_height(&self.inner, x, z, fa_mode)
    }

    fn block(&self, _x: usize, _y: isize, _z: usize) -> Option<&'static Block> {
        // Simplified: return AIR for all 1.13+ blocks
        Some(&AIR)
    }

    fn y_range(&self) -> std::ops::Range<isize> {
        fastanvil::Chunk::y_range(&self.inner)
    }
}

// ChunkData enum implementation - dispatches to the appropriate variant
impl Chunk for ChunkData {
    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        match self {
            ChunkData::Pre13(chunk) => chunk.surface_height(x, z, mode),
            ChunkData::Post13(chunk) => chunk.surface_height(x, z, mode),
        }
    }

    fn block(&self, x: usize, y: isize, z: usize) -> Option<&'static Block> {
        match self {
            ChunkData::Pre13(chunk) => chunk.block(x, y, z),
            ChunkData::Post13(chunk) => chunk.block(x, y, z),
        }
    }

    fn y_range(&self) -> std::ops::Range<isize> {
        match self {
            ChunkData::Pre13(chunk) => chunk.y_range(),
            ChunkData::Post13(chunk) => chunk.y_range(),
        }
    }
}
