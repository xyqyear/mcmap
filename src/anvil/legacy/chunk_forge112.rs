// Forge 1.10.x – 1.12.2 chunk parsing.
//
// This decoder handles two on-disk shapes that share the same NBT root:
//
//   * Vanilla pre-REI (Forge 1.10.x and any 1.12.x server without REI/JEID).
//     `Blocks` carries the low 8 bits of the block ID, optional `Add` is a
//     nibble array of high 4 bits, `Data` is the 4-bit metadata. No
//     `Palette` field.
//   * REI / JEID (RoughlyEnoughIDs / JustEnoughIDs). Reinterprets the same
//     `Blocks`/`Data` arrays as a 12-bit palette index (high 8 bits in
//     `Blocks`, low 4 bits in `Data`); a new `Palette` int-array maps the
//     index to `(block_id << 4) | meta`.
//
// `decode_section` picks the path on the presence of `Palette`. REI also
// writes chunk-level `Biomes` as `TAG_INT_ARRAY[256]` instead of vanilla's
// `TAG_BYTE_ARRAY[256]`; we accept both. Output shape is `LegacyChunkData` in
// either case so the existing legacy renderer takes both without changes.
// See `docs/forge_1_12_2_rei.md` for the full REI format reference.

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
    /// Vanilla pre-REI only: 2048 nibble bytes carrying the high 4 bits of
    /// the block ID. REI never emits `Add` — the high bits live in `Blocks`
    /// as part of the palette index.
    #[serde(rename = "Add", default)]
    add: Option<fastnbt::ByteArray>,
    /// REI: 2048 nibble bytes carrying the low 4 bits of the palette index.
    /// Vanilla pre-REI: 4-bit metadata.
    #[serde(rename = "Data", default)]
    data: Option<fastnbt::ByteArray>,
    /// REI-only: maps palette index → `(block_id << 4) | meta`. Absence of
    /// this field is the signal we're looking at a vanilla pre-REI section.
    #[serde(rename = "Palette", default)]
    palette: Option<IntArray>,
}

/// Parse a Forge 1.10.x – 1.12.2 chunk. Returns `LegacyChunkData` so the
/// existing renderer takes the result without changes.
pub fn from_bytes(data: &[u8]) -> Result<LegacyChunkData, String> {
    let root: ChunkRoot =
        fastnbt::from_bytes(data).map_err(|e| format!("Forge chunk NBT parse: {}", e))?;
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
    let SectionNbt {
        y,
        blocks,
        add,
        data,
        palette,
    } = sec;

    let blocks = match blocks {
        Some(b) if b.len() == SECTION_BLOCKS => b,
        Some(b) => {
            return Err(format!(
                "Blocks length {} (expected {})",
                b.len(),
                SECTION_BLOCKS
            ));
        }
        // No Blocks at all: treat as all-air. Reasonable for legacy "section
        // omitted" semantics; REI always writes Blocks alongside Palette.
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
        None => return Err("Section missing Data".into()),
    };

    let mut ids = Box::new([0u16; SECTION_BLOCKS]);
    let mut metas = Box::new([0u16; SECTION_BLOCKS]);

    match palette {
        Some(palette) => {
            // REI / JEID: 12-bit palette index packed across `Blocks` (high
            // 8) + `Data` (low 4). Palette entry is `(block_id << 4) | meta`.
            let palette_len = palette.len();
            for i in 0..SECTION_BLOCKS {
                let hi = blocks[i] as u8 as u16;
                let lo = nibble(&data[..], i) as u16;
                let pidx = ((hi << 4) | lo) as usize;
                if pidx >= palette_len {
                    // Out-of-range palette index — treat as air rather than
                    // panic. A corrupt chunk is no reason to abort the whole
                    // region render.
                    continue;
                }
                let state = palette[pidx] as u32;
                ids[i] = (state >> 4) as u16;
                metas[i] = (state & 0xF) as u16;
            }
        }
        None => {
            // Vanilla pre-REI: `Blocks` is the low 8 bits of the block ID,
            // optional `Add` nibble is the high 4 bits, `Data` nibble is the
            // 4-bit metadata.
            let add_bytes = add.as_ref().map(|a| a.as_ref());
            for i in 0..SECTION_BLOCKS {
                let lo_id = blocks[i] as u8 as u16;
                let hi_id = match add_bytes {
                    Some(a) if a.len() == SECTION_BLOCKS / 2 => nibble(a, i) as u16,
                    _ => 0,
                };
                ids[i] = (hi_id << 8) | lo_id;
                metas[i] = nibble(&data[..], i) as u16;
            }
        }
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
