// Forge 1.12.2 chunk parsing (RoughlyEnoughIDs / JustEnoughIDs format).
//
// REI keeps vanilla's `Sections[i].Blocks`/`Data` byte+nibble arrays but
// reinterprets their meaning: each block's encoded value is a 12-bit *palette
// index* (high 8 bits in `Blocks`, low 4 bits in `Data`). A new
// `Sections[i].Palette` int-array maps that index to a vanilla
// `IBlockState` id, encoded as `(block_id << 4) | meta`.
//
// REI also writes chunk-level `Biomes` as `TAG_INT_ARRAY[256]` instead of
// vanilla's `TAG_BYTE_ARRAY[256]`. We tolerate both so this parser also works
// for legacy chunks (saved before REI was installed).
//
// Output shape matches `LegacyChunkData` exactly so the existing legacy
// renderer takes both paths without modification. See
// `docs/forge_1_12_2_rei.md` for the full format reference.

use fastnbt::{IntArray, Value};
use serde::Deserialize;

use super::chunk::{LegacyChunkData, LegacySection, SECTION_BLOCKS};

/// NBT-facing root. Like `chunk::ChunkRoot` we tolerate sibling compounds at
/// the root (mods like `ForgeDataVersion`, `ImmersiveEngineering`,
/// `MekanismWorldGen`, …) by way of serde's default-ignore-unknown behavior.
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
    /// REI writes IntArray[256]; legacy chunks may have ByteArray[256] (or
    /// nothing). Deserialize as `Value` and decode in code so we don't fail
    /// the whole chunk on a type mismatch.
    #[serde(rename = "Biomes", default)]
    biomes: Option<Value>,
    #[serde(rename = "HeightMap", default)]
    heightmap: Option<IntArray>,
}

#[derive(Deserialize)]
struct SectionNbt {
    #[serde(rename = "Y")]
    y: i8,
    /// REI: 4096 bytes carrying the high 8 bits of the palette index.
    /// Vanilla pre-REI: low 8 bits of the block ID.
    #[serde(rename = "Blocks", default)]
    blocks: Option<fastnbt::ByteArray>,
    /// REI: 2048 nibble bytes carrying the low 4 bits of the palette index.
    /// Vanilla pre-REI: 4-bit metadata.
    #[serde(rename = "Data", default)]
    data: Option<fastnbt::ByteArray>,
    /// REI-only: maps palette index → `(block_id << 4) | meta`.
    #[serde(rename = "Palette", default)]
    palette: Option<IntArray>,
}

/// Parse a REI-format chunk. Returns `LegacyChunkData` so the existing
/// renderer accepts the result without changes.
pub fn from_bytes(data: &[u8]) -> Result<LegacyChunkData, String> {
    let root: ChunkRoot =
        fastnbt::from_bytes(data).map_err(|e| format!("REI chunk NBT parse: {}", e))?;
    let level = root.level;

    let mut sections: [Option<LegacySection>; 16] = Default::default();
    for sec in level.sections {
        let y = sec.y;
        let decoded = decode_section(sec)?;
        if let Some(decoded) = decoded {
            if (0..16).contains(&(y as i32)) {
                sections[y as usize] = Some(decoded);
            }
        }
    }

    let biomes = match level.biomes {
        Some(Value::IntArray(arr)) if arr.len() == 256 => {
            // REI writes one int per column; truncate to u8 for the cached
            // shape. (The renderer doesn't read biomes yet — this is forward-
            // compatibility only.)
            let mut out = [0u8; 256];
            for (i, v) in arr.iter().enumerate() {
                out[i] = (*v & 0xFF) as u8;
            }
            Some(out)
        }
        Some(Value::ByteArray(arr)) if arr.len() == 256 => {
            let mut out = [0u8; 256];
            for (i, v) in arr.iter().enumerate() {
                out[i] = *v as u8;
            }
            Some(out)
        }
        _ => None,
    };

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

fn decode_section(sec: SectionNbt) -> Result<Option<LegacySection>, String> {
    let SectionNbt { y, blocks, data, palette } = sec;

    let blocks = match blocks {
        Some(b) if b.len() == SECTION_BLOCKS => b,
        Some(b) => return Err(format!("Blocks length {} (expected {})", b.len(), SECTION_BLOCKS)),
        // No Blocks at all: treat as all-air. Reasonable for legacy "section
        // omitted" semantics, though REI always writes Blocks when Palette
        // exists.
        None => return Ok(None),
    };
    let data = match data {
        Some(d) if d.len() == SECTION_BLOCKS / 2 => d,
        Some(d) => {
            return Err(format!(
                "Data length {} (expected {})",
                d.len(),
                SECTION_BLOCKS / 2
            ));
        }
        None => return Err("Section missing Data — required by REI format".into()),
    };
    let palette = match palette {
        Some(p) => p,
        None => return Err("Section missing Palette — not a REI chunk".into()),
    };

    let mut ids = Box::new([0u16; SECTION_BLOCKS]);
    let mut metas = Box::new([0u16; SECTION_BLOCKS]);

    let palette_len = palette.len();
    for i in 0..SECTION_BLOCKS {
        let hi = blocks[i] as u8 as u16;
        let lo = nibble(&data[..], i) as u16;
        let pidx = ((hi << 4) | lo) as usize;
        if pidx >= palette_len {
            // Out-of-range palette index — treat as air rather than panic.
            // We've never seen this in practice, but a corrupt chunk is no
            // reason to abort the whole region render.
            continue;
        }
        let state = palette[pidx] as u32;
        ids[i] = (state >> 4) as u16;
        metas[i] = (state & 0xF) as u16;
    }

    Ok(Some(LegacySection { y, ids, metas }))
}

#[inline]
fn nibble(arr: &[i8], i: usize) -> u8 {
    let byte = arr[i >> 1] as u8;
    if i & 1 == 0 {
        byte & 0x0F
    } else {
        (byte >> 4) & 0x0F
    }
}
