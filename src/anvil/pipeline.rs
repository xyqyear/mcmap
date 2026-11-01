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
    type Boundary;
    type Decoder: Default;

    /// Retain a decoded northern seed without evaluating its heights.
    fn boundary(&self, chunk: Self::Chunk) -> Self::Boundary;

    /// Render one chunk and retain only what its southern neighbour needs.
    fn render_chunk(
        &self,
        chunk: Self::Chunk,
        north: Option<&Self::Boundary>,
    ) -> ([Rgba; 256], Self::Boundary);

    /// Parse a chunk's decompressed NBT bytes into the engine's chunk shape.
    /// Returning `Ok(None)` signals "treat as empty" (used when the engine
    /// wants to skip a malformed chunk without failing the whole region).
    fn decode(
        &self,
        decoder: &mut Self::Decoder,
        bytes: &[u8],
    ) -> Result<Option<Self::Chunk>, String>;
}

/// Drive one region through an engine. Returns `Ok(None)` when the region
/// file doesn't exist (caller decides whether to warn).
pub fn render_region<E: RenderEngine>(
    x: RCoord,
    z: RCoord,
    loader: &dyn RegionLoader,
    engine: &E,
) -> Result<Option<RegionMap>, String> {
    let mut decoder = E::Decoder::default();
    let mut map = RegionMap::new(x, z, [0u8; 4]);

    let mut region = match loader.region(x, z)? {
        Some(r) => r,
        None => return Ok(None),
    };

    // North-row cache: for top-shading continuity across the region boundary,
    // seed the "previous chunk" row with the southmost row of the region above.
    // Rendered chunks retain their southern heights; unused seeds stay lazy.
    let mut cache: [Option<E::Boundary>; 32] = Default::default();
    let mut bytes = Vec::new();
    if let Ok(Some(mut r)) = loader.region(x, RCoord(z.0 - 1)) {
        for (cx, entry) in cache.iter_mut().enumerate() {
            *entry = match r.read_chunk_into(cx, 31, &mut bytes) {
                Ok(true) => engine
                    .decode(&mut decoder, &bytes)
                    .ok()
                    .flatten()
                    .map(|chunk| engine.boundary(chunk)),
                _ => None,
            };
        }
    }

    for cz in 0usize..32 {
        for (cx, cache) in cache.iter_mut().enumerate() {
            let data = map.chunk_mut(CCoord(cx as isize), CCoord(cz as isize));

            let present = region
                .read_chunk_into(cx, cz, &mut bytes)
                .map_err(|e| format!("Failed to read chunk: {}", e))?;
            if !present {
                continue;
            }

            let chunk = match engine.decode(&mut decoder, &bytes) {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(e) => {
                    return Err(format!("Failed to decode chunk ({cx},{cz}): {e}"));
                }
            };

            let (rendered, boundary) = engine.render_chunk(chunk, cache.as_ref());
            data.copy_from_slice(&rendered);
            *cache = Some(boundary);
        }
    }

    Ok(Some(map))
}

#[cfg(test)]
mod tests;
