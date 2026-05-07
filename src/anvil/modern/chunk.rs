// 1.13+ chunk — wraps fastanvil's JavaChunk and corrects a heightmap-decode
// bug for 1.18+ saves.
//
// Vanilla 1.18+ sometimes serializes a section one step below the chunk's
// `yPos` (e.g. Y=-5 when yPos=-4) carrying light data with an empty or
// air-only blockstate palette. fastanvil 0.32 can't tell those apart from
// real sections (`Section::is_terminator` returns `false` unconditionally
// post-1.18) and ends up with `SectionTower::y_min = -80` instead of -64.
// `expand_heightmap` then reads packed values 16 blocks too low, and the
// renderer's drill-for-colour walks past the actual surface — visible as
// tinted-grass pixels where solid blocks should be.
//
// We work around it without forking fastanvil: detect the offset, re-expand
// the heightmap with the correct `y_min`, and shadow `surface_height` /
// `y_range` with the corrected values via our own `Chunk` impl.

use std::ops::Range;

pub use fastanvil::HeightMode;
use fastanvil::{Block, Chunk, JavaChunk, biome::Biome, expand_heightmap};

pub struct ChunkData {
    inner: JavaChunk,
    corrected: Option<Corrected>,
}

struct Corrected {
    y_min: isize,
    y_max: isize,
    heightmap: [i16; 256],
}

impl ChunkData {
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let inner = JavaChunk::from_bytes(data).map_err(|e| format!("Failed to parse chunk: {}", e))?;
        let corrected = compute_correction(&inner);
        Ok(Self { inner, corrected })
    }

    pub fn inner(&self) -> &JavaChunk {
        &self.inner
    }
}

fn compute_correction(chunk: &JavaChunk) -> Option<Corrected> {
    let cur = match chunk {
        JavaChunk::Post18(c) => c,
        _ => return None,
    };
    let sections = cur.sections.as_ref()?;

    let lowest_real = sections
        .sections()
        .iter()
        .filter(|s| {
            let pal = s.block_states.palette();
            !pal.is_empty() && !pal.iter().all(|b| b.name() == "minecraft:air")
        })
        .min_by_key(|s| s.y)?;
    let real_y_min = (lowest_real.y as isize) * 16;

    if real_y_min == sections.y_min() {
        return None;
    }

    let hm = cur.heightmaps.as_ref()?.motion_blocking.as_ref()?;
    let expanded = expand_heightmap(hm, real_y_min, cur.data_version);
    let mut heightmap = [0i16; 256];
    for (i, v) in expanded.iter().take(256).enumerate() {
        heightmap[i] = *v;
    }

    Some(Corrected {
        y_min: real_y_min,
        y_max: sections.y_max(),
        heightmap,
    })
}

impl Chunk for ChunkData {
    fn status(&self) -> String {
        self.inner.status()
    }

    fn surface_height(&self, x: usize, z: usize, mode: HeightMode) -> isize {
        if let Some(c) = &self.corrected
            && matches!(mode, HeightMode::Trust)
        {
            return c.heightmap[z * 16 + x] as isize;
        }
        self.inner.surface_height(x, z, mode)
    }

    fn biome(&self, x: usize, y: isize, z: usize) -> Option<Biome> {
        self.inner.biome(x, y, z)
    }

    fn block(&self, x: usize, y: isize, z: usize) -> Option<&Block> {
        self.inner.block(x, y, z)
    }

    fn y_range(&self) -> Range<isize> {
        if let Some(c) = &self.corrected {
            c.y_min..c.y_max
        } else {
            self.inner.y_range()
        }
    }
}
