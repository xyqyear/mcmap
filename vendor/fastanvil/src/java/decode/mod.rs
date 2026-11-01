mod cache;
mod roots;
mod sections;
#[cfg(test)]
mod tests;

use crate::JavaChunk;
use cache::BlockCache;
use mcmap_nbt::{Error, Result};

/// Own one decoder per region to reuse palette entries without retaining world data.
#[derive(Default)]
pub struct ChunkDecoder {
    cache: BlockCache,
}

impl ChunkDecoder {
    pub fn decode(&mut self, bytes: &[u8]) -> Result<JavaChunk> {
        roots::decode(bytes, &mut self.cache).or_else(|_| JavaChunk::from_bytes_generic(bytes))
    }
}

fn bad() -> Error {
    Error("incompatible chunk shape".into())
}
fn compound(tag: u8) -> Result<()> {
    if tag == 10 {
        Ok(())
    } else {
        Err(bad())
    }
}
fn seen(bits: &mut u32, bit: u32) -> Result<()> {
    if *bits & bit != 0 {
        return Err(bad());
    }
    *bits |= bit;
    Ok(())
}
