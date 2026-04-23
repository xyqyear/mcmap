// Region-level rendering pipeline — shared across every on-disk chunk format.
//
// The region file layout (32×32 chunks per .mca, zlib-framed payloads,
// north-row cache for top-shading across region boundaries) is identical
// between 1.13+, 1.7.10, and the Forge 1.12.2 REI variants. The only
// per-version work is decoding a chunk blob and turning a decoded chunk
// into a 16×16 Rgba column — that's what `RenderEngine` abstracts.

use super::region::{CCoord, RCoord, RegionLoader};

pub type Rgba = [u8; 4];

/// Rendered region: a flat 32×32-of-16×16 Rgba grid (YZX within each chunk).
pub struct RegionMap {
    pub x: RCoord,
    pub z: RCoord,
    data: Vec<Rgba>,
}

impl RegionMap {
    pub fn new(x: RCoord, z: RCoord, fill: Rgba) -> Self {
        let len = 32 * 16 * 32 * 16;
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

/// Version-specific chunk decode + rendering.
///
/// The pipeline owns region I/O, neighbour caching, and RegionMap allocation;
/// the engine just converts chunk bytes into a 16×16 Rgba column and decides
/// which NBT schema to use.
pub trait RenderEngine {
    type Chunk;

    /// Parse a chunk's decompressed NBT bytes into the engine's chunk shape.
    /// Returning `Ok(None)` signals "treat as empty" (used when the engine
    /// wants to skip a malformed chunk without failing the whole region).
    fn decode(&self, bytes: &[u8]) -> Result<Option<Self::Chunk>, String>;

    /// Render a single chunk to 16×16 pixels. `north` is the chunk immediately
    /// to the north (smaller z) in world coords — used for top-shading. When
    /// the current chunk is at the top of a region, `north` may be the last
    /// row of the region above; when the region above is missing, it's `None`.
    fn render_chunk(
        &self,
        chunk: &Self::Chunk,
        north: Option<&Self::Chunk>,
    ) -> [Rgba; 16 * 16];
}

/// Drive one region through an engine. Returns `Ok(None)` when the region
/// file doesn't exist (caller decides whether to warn).
pub fn render_region<E: RenderEngine>(
    x: RCoord,
    z: RCoord,
    loader: &dyn RegionLoader,
    engine: &E,
) -> Result<Option<RegionMap>, String> {
    let mut map = RegionMap::new(x, z, [0u8; 4]);

    let mut region = match loader.region(x, z)? {
        Some(r) => r,
        None => return Ok(None),
    };

    // North-row cache: for top-shading continuity across the region boundary,
    // seed the "previous chunk" row with the southmost row of the region above.
    // The cache also holds chunks from within the current region for per-row
    // chaining.
    let mut cache: [Option<E::Chunk>; 32] = Default::default();
    if let Ok(Some(mut r)) = loader.region(x, RCoord(z.0 - 1)) {
        for (cx, entry) in cache.iter_mut().enumerate() {
            *entry = r
                .read_chunk(cx, 31)
                .ok()
                .flatten()
                .and_then(|b| engine.decode(&b).ok().flatten());
        }
    }

    for cz in 0usize..32 {
        for (cx, cache) in cache.iter_mut().enumerate() {
            let data = map.chunk_mut(CCoord(cx as isize), CCoord(cz as isize));

            let bytes = region
                .read_chunk(cx, cz)
                .map_err(|e| format!("Failed to read chunk: {}", e))?;
            let Some(bytes) = bytes else {
                continue;
            };

            let chunk = match engine.decode(&bytes) {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(e) => {
                    log::warn!("Skipping malformed chunk ({},{}): {}", cx, cz, e);
                    continue;
                }
            };

            let rendered = engine.render_chunk(&chunk, cache.as_ref());
            data.copy_from_slice(&rendered);
            *cache = Some(chunk);
        }
    }

    Ok(Some(map))
}
