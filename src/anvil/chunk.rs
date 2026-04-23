// Chunk data — 1.13+ only. Wraps fastanvil's JavaChunk.

pub use fastanvil::HeightMode;

#[derive(Debug)]
pub struct ChunkData {
    inner: fastanvil::JavaChunk,
}

impl ChunkData {
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        fastanvil::JavaChunk::from_bytes(data)
            .map(|inner| Self { inner })
            .map_err(|e| format!("Failed to parse chunk: {}", e))
    }

    pub fn inner(&self) -> &fastanvil::JavaChunk {
        &self.inner
    }
}
