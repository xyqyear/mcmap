// 1.13+ chunk — wraps fastanvil's `JavaChunk` and adapts surface-height
// reporting around two unrelated quirks of fastanvil 0.32:
//
//   1. The 1.18+ "terminator section" bug. Vanilla 1.18+ sometimes serializes
//      a section one step below the chunk's `yPos` (e.g. Y=-5 when yPos=-4)
//      carrying light data with an empty or air-only blockstate palette.
//      `Section::is_terminator` returns `false` unconditionally post-1.18, so
//      fastanvil treats the spurious section as real, ends up with
//      `SectionTower::y_min = -80` instead of -64, and `expand_heightmap`
//      reads packed values 16 blocks too low. The renderer then drills past
//      the actual surface — visible as tinted-grass pixels where solid blocks
//      should be. We re-expand the heightmap with the correct `y_min` and
//      shadow `surface_height` / `y_range` with the corrected values.
//
//   2. Modded heightmap packings. fastanvil's `expand_heightmap` hard-codes
//      only the heightmap lengths vanilla emits (36/37 longs pre-1.17, 37/43
//      from 1.17 on). Mods that ship dims with non-standard Y ranges write
//      heightmaps with whichever bits/item the range needs (mine cells:
//      29/32 longs; petconnect tall dims: 52 longs). Calling `expand_heightmap`
//      on those panics. The same call sits behind fastanvil's own
//      `surface_height(_, _, Trust)`, so we have to bypass `Trust` entirely
//      for these chunks.
//
// `HeightmapStrategy` captures both corrections in one state.

use std::ops::Range;

pub use fastanvil::HeightMode;
use fastanvil::{Block, Chunk, JavaChunk, biome::Biome, expand_heightmap};

pub struct ChunkData {
    inner: JavaChunk,
    strategy: HeightmapStrategy,
}

/// How to answer `surface_height` and `y_range` for this chunk.
enum HeightmapStrategy {
    /// Defer entirely to fastanvil — the standard case.
    Inner,
    /// Use the precomputed heightmap for `Trust`, plus override `y_range`
    /// with the corrected bounds. Fixes the 1.18+ terminator-section bug.
    Corrected {
        y_min: isize,
        y_max: isize,
        heightmap: [i16; 256],
    },
    /// Force `HeightMode::Calculate` on every call — fastanvil's
    /// `expand_heightmap` would panic on this chunk's heightmap length.
    /// `Calculate` walks blocks directly and never touches the broken
    /// expander. Slightly slower per chunk but it ships a real picture.
    ForceCalculate,
}

impl ChunkData {
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let inner =
            JavaChunk::from_bytes(data).map_err(|e| format!("Failed to parse chunk: {}", e))?;
        let strategy = HeightmapStrategy::from_chunk(&inner);
        Ok(Self { inner, strategy })
    }

    pub fn inner(&self) -> &JavaChunk {
        &self.inner
    }
}

impl HeightmapStrategy {
    fn from_chunk(chunk: &JavaChunk) -> Self {
        let post = match chunk {
            JavaChunk::Post18(c) => c,
            _ => return Self::Inner,
        };

        // If the heightmap length is one fastanvil's `expand_heightmap` can't
        // handle, every Trust path through fastanvil panics — including its
        // own `surface_height(_, _, Trust)`. Bail out before we touch any
        // expander and let the renderer fall back to `Calculate`.
        let hm = post
            .heightmaps
            .as_ref()
            .and_then(|h| h.motion_blocking.as_ref());
        if let Some(hm) = hm
            && !is_known_heightmap_len(hm.len(), post.data_version)
        {
            return Self::ForceCalculate;
        }

        // Detect the terminator-section offset: a "real" section is any
        // non-empty palette that has at least one non-air block. If the
        // lowest real section sits above the tower's y_min, fastanvil counted
        // a phantom section and the heightmap is mis-aligned.
        let Some(sections) = post.sections.as_ref() else {
            return Self::Inner;
        };
        let Some(lowest_real) = sections
            .sections()
            .iter()
            .filter(|s| {
                let pal = s.block_states.palette();
                !pal.is_empty() && !pal.iter().all(|b| b.name() == "minecraft:air")
            })
            .min_by_key(|s| s.y)
        else {
            return Self::Inner;
        };
        let real_y_min = (lowest_real.y as isize) * 16;
        if real_y_min == sections.y_min() {
            return Self::Inner;
        }

        let Some(hm) = hm else {
            return Self::Inner;
        };
        // Length already validated above.
        let expanded = expand_heightmap(hm, real_y_min, post.data_version);
        let mut heightmap = [0i16; 256];
        for (i, v) in expanded.iter().take(256).enumerate() {
            heightmap[i] = *v;
        }
        Self::Corrected {
            y_min: real_y_min,
            y_max: sections.y_max(),
            heightmap,
        }
    }
}

/// Mirror of fastanvil 0.32 `expand_heightmap`'s accepted shapes. Anything
/// outside this set crashes that function with a hard-coded `panic!`.
///
/// Pre-1.17:  36 longs (1.15, 9 bits/item, no padding) or 37 longs (1.16+,
///            9 bits/item with one padding bit per long).
/// 1.17 on:   37 longs (9 bits/item) or 43 longs (10 bits/item, taller world).
fn is_known_heightmap_len(len: usize, data_version: i32) -> bool {
    const V1_17_0: i32 = 2724;
    if data_version >= V1_17_0 {
        len == 37 || len == 43
    } else {
        matches!(len, 36 | 37)
    }
}

impl Chunk for ChunkData {
    fn status(&self) -> String {
        self.inner.status()
    }

    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        match (&self.strategy, mode) {
            (HeightmapStrategy::Corrected { heightmap, .. }, HeightMode::Trust) => {
                heightmap[z * 16 + x] as isize
            }
            (HeightmapStrategy::ForceCalculate, _) => {
                self.inner.surface_height(x, z, HeightMode::Calculate)
            }
            _ => self.inner.surface_height(x, z, mode),
        }
    }

    fn biome(&self, x: usize, y: isize, z: usize) -> Option<Biome> {
        self.inner.biome(x, y, z)
    }

    fn block(&self, x: usize, y: isize, z: usize) -> Option<&Block> {
        self.inner.block(x, y, z)
    }

    fn y_range(&self) -> Range<isize> {
        if let HeightmapStrategy::Corrected { y_min, y_max, .. } = &self.strategy {
            *y_min..*y_max
        } else {
            self.inner.y_range()
        }
    }
}
