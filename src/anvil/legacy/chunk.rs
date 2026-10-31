// Pre-1.13 chunk NBT parsing.

use fastnbt::{ByteArray, IntArray};
use serde::Deserialize;

pub const SECTION_BLOCKS: usize = 16 * 16 * 16; // 4096

/// NBT-facing shape of a chunk. Extra root-level compounds (Thaumcraft,
/// CoFHWorld, …) in GTNH worlds are ignored by serde's default behavior.
#[derive(Deserialize)]
struct ChunkRoot {
    #[serde(rename = "Level")]
    level: Level,
    #[serde(rename = "DataVersion", default)]
    #[allow(dead_code)]
    data_version: Option<i32>,
}

#[derive(Deserialize)]
struct Level {
    #[serde(rename = "xPos")]
    x_pos: i32,
    #[serde(rename = "zPos")]
    z_pos: i32,
    #[serde(rename = "Sections", default)]
    sections: Vec<SectionNbt>,
    #[serde(rename = "Biomes", default)]
    biomes: Option<ByteArray>,
    #[serde(rename = "HeightMap", default)]
    heightmap: Option<IntArray>,
}

#[derive(Deserialize)]
struct SectionNbt {
    #[serde(rename = "Y")]
    y: i8,
    // NEID-extended (preferred when present).
    #[serde(rename = "Blocks16", default)]
    blocks16: Option<ByteArray>, // 8192 bytes = 4096 big-endian u16
    #[serde(rename = "Data16", default)]
    data16: Option<ByteArray>, // 8192 bytes = 4096 big-endian u16
    // Vanilla (fallback or NEID's PostNeidWorldsSupport compatibility copy).
    #[serde(rename = "Blocks", default)]
    blocks: Option<ByteArray>, // 4096 bytes, u8 low byte of id
    #[serde(rename = "Add", default)]
    add: Option<ByteArray>, // 2048 bytes (nibble-packed high 4 bits)
    #[serde(rename = "Data", default)]
    data: Option<ByteArray>, // 2048 bytes (nibble-packed metadata)
}

/// Decoded section: a flat 4096-entry (YZX) array of (id, meta) pairs.
pub struct LegacySection {
    #[allow(dead_code)] // kept for symmetry with NBT; indexing uses the parent array
    pub y: i8,
    pub ids: Box<[u16; SECTION_BLOCKS]>,
    pub metas: Box<[u16; SECTION_BLOCKS]>,
}

impl LegacySection {
    fn from_nbt(sec: SectionNbt) -> Result<Self, String> {
        let mut ids = Box::new([0u16; SECTION_BLOCKS]);
        let mut metas = Box::new([0u16; SECTION_BLOCKS]);

        // NEID: `Blocks16` is 4096 big-endian u16s. When present it fully
        // supersedes Blocks+Add.
        if let Some(b16) = sec.blocks16.as_deref() {
            if b16.len() != SECTION_BLOCKS * 2 {
                return Err(format!(
                    "Blocks16 length {} (expected {})",
                    b16.len(),
                    SECTION_BLOCKS * 2
                ));
            }
            for i in 0..SECTION_BLOCKS {
                let hi = b16[2 * i] as u8;
                let lo = b16[2 * i + 1] as u8;
                ids[i] = u16::from_be_bytes([hi, lo]);
            }
        } else if let Some(blocks) = sec.blocks.as_deref() {
            if blocks.len() != SECTION_BLOCKS {
                return Err(format!(
                    "Blocks length {} (expected {})",
                    blocks.len(),
                    SECTION_BLOCKS
                ));
            }
            if let Some(add) = sec.add.as_deref() {
                if add.len() != SECTION_BLOCKS / 2 {
                    return Err(format!(
                        "Add length {} (expected {})",
                        add.len(),
                        SECTION_BLOCKS / 2
                    ));
                }
                unpack_ids_with_add(&mut ids, blocks, add);
            } else {
                for i in 0..SECTION_BLOCKS {
                    ids[i] = blocks[i] as u8 as u16;
                }
            }
        } else {
            // No block data at all — treat as all air. Reasonable since Anvil
            // omits air-only sections, but we may hit this on corrupted files.
            // ids stays zeroed.
        }

        // NEID `Data16` supersedes vanilla `Data`.
        if let Some(d16) = sec.data16.as_deref() {
            if d16.len() != SECTION_BLOCKS * 2 {
                return Err(format!(
                    "Data16 length {} (expected {})",
                    d16.len(),
                    SECTION_BLOCKS * 2
                ));
            }
            for i in 0..SECTION_BLOCKS {
                let hi = d16[2 * i] as u8;
                let lo = d16[2 * i + 1] as u8;
                metas[i] = u16::from_be_bytes([hi, lo]);
            }
        } else if let Some(data) = sec.data.as_deref() {
            if data.len() != SECTION_BLOCKS / 2 {
                return Err(format!(
                    "Data length {} (expected {})",
                    data.len(),
                    SECTION_BLOCKS / 2
                ));
            }
            unpack_nibbles(&mut metas, data);
        }

        Ok(LegacySection {
            y: sec.y,
            ids,
            metas,
        })
    }
}

pub(super) fn unpack_ids_with_add(ids: &mut [u16; SECTION_BLOCKS], blocks: &[i8], add: &[i8]) {
    for i in 0..SECTION_BLOCKS / 2 {
        let high = add[i] as u8 as u16;
        ids[2 * i] = blocks[2 * i] as u8 as u16 | ((high & 0xF) << 8);
        ids[2 * i + 1] = blocks[2 * i + 1] as u8 as u16 | ((high >> 4) << 8);
    }
}

pub(super) fn unpack_nibbles(values: &mut [u16; SECTION_BLOCKS], packed: &[i8]) {
    for i in 0..SECTION_BLOCKS / 2 {
        let byte = packed[i] as u8;
        values[2 * i] = (byte & 0xF) as u16;
        values[2 * i + 1] = (byte >> 4) as u16;
    }
}

/// A decoded pre-1.13 chunk. Sections are stored by section-Y index (0..15);
/// missing sections (no storage written) are `None`.
pub struct LegacyChunkData {
    #[allow(dead_code)]
    pub x_pos: i32,
    #[allow(dead_code)]
    pub z_pos: i32,
    pub sections: [Option<LegacySection>; 16],
    #[allow(dead_code)] // exposed for future biome-aware rendering
    pub biomes: Option<[u8; 256]>,
    pub heightmap: Option<[i32; 256]>,
}

impl LegacyChunkData {
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let root: ChunkRoot = fastnbt::from_bytes(data).map_err(|e| format!("NBT parse: {}", e))?;
        let level = root.level;

        let mut sections: [Option<LegacySection>; 16] = Default::default();
        for sec in level.sections {
            let y = sec.y;
            let decoded = LegacySection::from_nbt(sec)?;
            if (0..16).contains(&(y as i32)) {
                sections[y as usize] = Some(decoded);
            }
            // y outside 0..15 is valid for nether (0..7) — falls inside the
            // range anyway. Negative or >15 sections are ignored silently.
        }

        let biomes = level.biomes.and_then(|b| {
            if b.len() == 256 {
                let mut out = [0u8; 256];
                for (i, v) in b.iter().enumerate() {
                    out[i] = *v as u8;
                }
                Some(out)
            } else {
                None
            }
        });

        let heightmap = level.heightmap.and_then(|h| {
            if h.len() == 256 {
                let mut out = [0i32; 256];
                out.copy_from_slice(&h[..]);
                Some(out)
            } else {
                None
            }
        });

        Ok(LegacyChunkData {
            x_pos: level.x_pos,
            z_pos: level.z_pos,
            sections,
            biomes,
            heightmap,
        })
    }

    /// Block ID + metadata at local (x, y, z). Returns `(0, 0)` (air) outside
    /// the vertical range or for sections that weren't written.
    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> (u16, u16) {
        if y >= 256 {
            return (0, 0);
        }
        let sec_y = y >> 4;
        let Some(sec) = self.sections[sec_y].as_ref() else {
            return (0, 0);
        };
        let i = ((y & 0xF) << 8) | (z << 4) | x;
        (sec.ids[i], sec.metas[i])
    }

    /// Heightmap value at local (x, z) — "first Y at which skylight is full",
    /// i.e. Y above the highest light-blocker. Clamped to 0..255 on return.
    #[inline]
    pub fn heightmap_at(&self, x: usize, z: usize) -> Option<i32> {
        self.heightmap.as_ref().map(|h| h[z * 16 + x])
    }
}

/// Peek at a decompressed chunk NBT payload to decide whether to use the
/// legacy or modern parsing path. Returns true iff the chunk lacks
/// `DataVersion` or has one strictly below 1451 (1.13 release).
///
/// Currently unused — the render command picks the path from the palette
/// format, not per-chunk. Kept available for callers that want per-chunk
/// dispatch (e.g. a future mixed-world analyzer).
#[allow(dead_code)]
pub fn is_legacy_chunk(data: &[u8]) -> Result<bool, String> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(rename = "DataVersion", default)]
        data_version: Option<i32>,
    }
    let probe: Probe = fastnbt::from_bytes(data).map_err(|e| format!("NBT probe: {}", e))?;
    Ok(match probe.data_version {
        None => true,
        Some(v) => v < 1451,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> SectionNbt {
        SectionNbt {
            y: 0,
            blocks16: None,
            data16: None,
            blocks: None,
            add: None,
            data: None,
        }
    }

    #[test]
    fn packed_ids_and_metadata_keep_every_even_and_odd_block() {
        let mut blocks = vec![0i8; SECTION_BLOCKS];
        let mut add = vec![0i8; SECTION_BLOCKS / 2];
        let mut data = vec![0i8; SECTION_BLOCKS / 2];
        for index in 0..SECTION_BLOCKS {
            let id = ((index * 911 + 0xABC) & 0xFFF) as u16;
            let meta = ((index * 7 + 3) & 0xF) as u8;
            blocks[index] = id as i8;
            add[index / 2] |= (((id >> 8) as u8) << (4 * (index % 2))) as i8;
            data[index / 2] |= (meta << (4 * (index % 2))) as i8;
        }
        let decoded = LegacySection::from_nbt(SectionNbt {
            blocks: Some(ByteArray::new(blocks)),
            add: Some(ByteArray::new(add)),
            data: Some(ByteArray::new(data)),
            ..section()
        })
        .unwrap();
        for index in 0..SECTION_BLOCKS {
            assert_eq!(decoded.ids[index], ((index * 911 + 0xABC) & 0xFFF) as u16);
            assert_eq!(decoded.metas[index], ((index * 7 + 3) & 0xF) as u16);
        }
    }

    #[test]
    fn extended_fields_override_invalid_vanilla_arrays_and_keep_wide_metadata() {
        let mut blocks16 = Vec::with_capacity(SECTION_BLOCKS * 2);
        let mut data16 = Vec::with_capacity(SECTION_BLOCKS * 2);
        for index in 0..SECTION_BLOCKS {
            blocks16.extend((index as u16 ^ 0xFEDC).to_be_bytes().map(|b| b as i8));
            data16.extend((index as u16 ^ 0xABCD).to_be_bytes().map(|b| b as i8));
        }
        let decoded = LegacySection::from_nbt(SectionNbt {
            blocks16: Some(ByteArray::new(blocks16)),
            data16: Some(ByteArray::new(data16)),
            blocks: Some(ByteArray::new(vec![0])),
            add: Some(ByteArray::new(vec![0])),
            data: Some(ByteArray::new(vec![0])),
            ..section()
        })
        .unwrap();
        for index in 0..SECTION_BLOCKS {
            assert_eq!(decoded.ids[index], index as u16 ^ 0xFEDC);
            assert_eq!(decoded.metas[index], index as u16 ^ 0xABCD);
        }
    }

    #[test]
    fn invalid_add_is_rejected_before_invalid_data() {
        let result = LegacySection::from_nbt(SectionNbt {
            blocks: Some(ByteArray::new(vec![0; SECTION_BLOCKS])),
            add: Some(ByteArray::new(vec![0])),
            data: Some(ByteArray::new(vec![0])),
            ..section()
        });
        assert!(matches!(result, Err(error) if error == "Add length 1 (expected 2048)"));
    }
}
