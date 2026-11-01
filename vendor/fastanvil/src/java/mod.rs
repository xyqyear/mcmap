use std::ops::Range;

use mcmap_nbt::{error::Result, from_bytes};
/// 1.2 to 1.12
pub mod pre13;
/// 1.13 to 1.17
pub mod pre18;

mod block;
mod chunk;
mod decode;
mod default_states_26_3;
mod heightmaps;
#[cfg(test)]
mod routing_tests;
pub use decode::ChunkDecoder;
mod section;
mod section_data;
mod section_tower;

pub use block::*;
pub use chunk::*;
pub use heightmaps::*;
pub use section::*;
pub use section_data::*;
pub use section_tower::*;

use once_cell::sync::Lazy;

use crate::{biome::Biome, Chunk, HeightMode};

pub static AIR: Lazy<Block> = Lazy::new(|| Block {
    name: "minecraft:air".into(),
    encoded: "minecraft:air|".into(),
    archetype: BlockArchetype::Airy,
});
pub static SNOW_BLOCK: Lazy<Block> = Lazy::new(|| Block {
    name: "minecraft:snow_block".into(),
    encoded: "minecraft:snow_block|".into(),
    archetype: BlockArchetype::Snowy,
});

/// A Minecraft chunk.
#[derive(Debug)]
pub enum JavaChunk {
    Post18(CurrentJavaChunk),
    Pre18(pre18::JavaChunk),
    Pre13(pre13::JavaChunk),
}

impl JavaChunk {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        ChunkDecoder::default().decode(data)
    }

    fn from_bytes_generic(data: &[u8]) -> Result<Self> {
        let chunk: Result<CurrentJavaChunk> = from_bytes(data);

        match chunk {
            Ok(chunk) => Ok(Self::Post18(chunk)),
            Err(modern_error) => match from_bytes::<pre18::JavaChunk>(data) {
                Ok(chunk) => Ok(Self::Pre18(chunk)),
                Err(pre18_error) => from_bytes::<pre13::JavaChunk>(data)
                    .map(Self::Pre13)
                    .map_err(|pre13_error| serde::de::Error::custom(format!(
                        "Invalid chunk: modern: {modern_error}; pre-1.18: {pre18_error}; pre-1.13: {pre13_error}"
                    ))),
            },
        }
    }
}

// TODO: Find a better way to dispatch these methods.
impl Chunk for JavaChunk {
    fn status(&self) -> String {
        match self {
            JavaChunk::Post18(c) => c.status(),
            JavaChunk::Pre18(c) => c.status(),
            JavaChunk::Pre13(c) => c.status(),
        }
    }

    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        match self {
            JavaChunk::Post18(c) => c.surface_height(x, z, mode),
            JavaChunk::Pre18(c) => c.surface_height(x, z, mode),
            JavaChunk::Pre13(c) => c.surface_height(x, z, mode),
        }
    }

    fn surface_heights(&self, mode: HeightMode) -> [isize; 256] {
        match self {
            JavaChunk::Post18(c) => c.surface_heights(mode),
            JavaChunk::Pre18(c) => c.surface_heights(mode),
            JavaChunk::Pre13(c) => c.surface_heights(mode),
        }
    }

    fn biome(&self, x: usize, y: isize, z: usize) -> Option<Biome> {
        match self {
            JavaChunk::Post18(c) => c.biome(x, y, z),
            JavaChunk::Pre18(c) => c.biome(x, y, z),
            JavaChunk::Pre13(c) => c.biome(x, y, z),
        }
    }

    fn block(&self, x: usize, y: isize, z: usize) -> Option<&Block> {
        match self {
            JavaChunk::Post18(c) => c.block(x, y, z),
            JavaChunk::Pre18(c) => c.block(x, y, z),
            JavaChunk::Pre13(c) => c.block(x, y, z),
        }
    }

    fn water_depth(&self, x: usize, y: isize, z: usize, y_min: isize) -> isize {
        match self {
            JavaChunk::Post18(c) => c.water_depth(x, y, z, y_min),
            JavaChunk::Pre18(c) => c.water_depth(x, y, z, y_min),
            JavaChunk::Pre13(c) => c.water_depth(x, y, z, y_min),
        }
    }

    fn y_range(&self) -> Range<isize> {
        match self {
            JavaChunk::Post18(c) => c.y_range(),
            JavaChunk::Pre18(c) => c.y_range(),
            JavaChunk::Pre13(c) => c.y_range(),
        }
    }
}
