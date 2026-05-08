// Unified `level.dat` parser + variant detection for `gen-palette`.
//
// Two pre-1.13 FML formats coexist; both live in the same `level.dat`
// gzip-compressed NBT, just under different keys:
//
//   - 1.7.10: `FML.ItemData` is a `TAG_List<Compound>` of `{K: "<prefix><name>",
//     V: int_id}` where the prefix byte is `\x01` for blocks and `\x02` for
//     items. Some entries have empty/malformed K — skipped silently.
//
//   - 1.8+ (and 1.12.2 + REI/JEID): `FML.Registries.<reg_name>.ids` —
//     `List<{K: "ns:name", V: int_id}>`. Block ids live under
//     `minecraft:blocks`. The id range is uncapped under REI/JEID, so we
//     treat `V` as a full `i32` and cast to `u32`.
//
// `detect_variant(level_dat)` peeks at FML once to pick the right pipeline:
//   - `FML.Registries.minecraft:blocks` present → Forge112
//   - else `FML.ItemData` present              → Legacy17
//   - else (or no level.dat at all)            → Modern

use flate2::read::GzDecoder;
use log::debug;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::PaletteVariant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
pub struct FmlRegistry17 {
    pub blocks: HashMap<u16, String>,
    pub items: HashMap<u16, String>,
}

#[derive(Default)]
pub struct FmlRegistry12 {
    /// Numeric block id → `"namespace:name"`. Range is `0 .. ~28-bit` under
    /// REI; in practice modpacks stay under ~10000.
    pub blocks: HashMap<u32, String>,
}

/// Probe shape — used only for `detect_variant`. Both `ItemData` and
/// `Registries` are deserialized as `serde_json::Value`-ish leaves so we don't
/// pay for full parsing during detection.
#[derive(Deserialize)]
struct LevelDatProbe {
    #[serde(rename = "FML", default)]
    fml: Option<FmlProbe>,
}

#[derive(Deserialize)]
struct FmlProbe {
    #[serde(rename = "ItemData", default)]
    item_data: Option<fastnbt::Value>,
    #[serde(rename = "Registries", default)]
    registries: Option<RegistriesProbe>,
}

#[derive(Deserialize)]
struct RegistriesProbe {
    #[serde(rename = "minecraft:blocks", default)]
    blocks: Option<fastnbt::Value>,
}

/// Detection-only entrypoint. `None` → Modern (the only variant that doesn't
/// need a level.dat). `Some(path)` → read FML once and pick:
///   - Registries.minecraft:blocks present → Forge112
///   - else ItemData present               → Legacy17
///   - else                                → Modern (vanilla 1.13+ level.dat)
pub fn detect_variant(level_dat: Option<&Path>) -> Result<PaletteVariant> {
    let Some(path) = level_dat else {
        return Ok(PaletteVariant::Modern);
    };
    let nbt = read_gzipped(path)?;
    let probe: LevelDatProbe = fastnbt::from_bytes(&nbt)
        .map_err(|e| format!("level.dat NBT parse: {}", e))?;
    let Some(fml) = probe.fml else {
        return Ok(PaletteVariant::Modern);
    };
    if fml.registries.as_ref().and_then(|r| r.blocks.as_ref()).is_some() {
        return Ok(PaletteVariant::Forge112);
    }
    if fml.item_data.is_some() {
        return Ok(PaletteVariant::Legacy);
    }
    Ok(PaletteVariant::Modern)
}

// ----- 1.7.10 parser -----------------------------------------------------

#[derive(Deserialize)]
struct LevelDatV17 {
    #[serde(rename = "FML")]
    fml: FmlV17,
}

#[derive(Deserialize)]
struct FmlV17 {
    #[serde(rename = "ItemData", default)]
    item_data: Vec<ItemEntry17>,
}

#[derive(Deserialize)]
struct ItemEntry17 {
    #[serde(rename = "K", default)]
    k: Option<String>,
    #[serde(rename = "V", default)]
    v: Option<i32>,
}

pub fn load_fml_registry_v17(path: &Path) -> Result<FmlRegistry17> {
    let nbt = read_gzipped(path)?;
    let data: LevelDatV17 = fastnbt::from_bytes(&nbt)
        .map_err(|e| format!("level.dat NBT parse: {}", e))?;

    let mut registry = FmlRegistry17::default();
    for entry in data.fml.item_data {
        let (Some(key), Some(id)) = (entry.k, entry.v) else {
            continue;
        };
        if key.is_empty() || id < 0 || id > u16::MAX as i32 {
            continue;
        }
        let id = id as u16;
        let first_byte = key.as_bytes()[0];
        let name = &key[1..];
        match first_byte {
            0x01 => {
                registry.blocks.insert(id, name.to_string());
            }
            0x02 => {
                registry.items.insert(id, name.to_string());
            }
            _ => {
                debug!("Unknown FML registry prefix: {:#x} for {:?}", first_byte, key);
            }
        }
    }
    Ok(registry)
}

// ----- 1.12.2 parser -----------------------------------------------------

#[derive(Deserialize)]
struct LevelDatV12 {
    #[serde(rename = "FML")]
    fml: FmlV12,
}

#[derive(Deserialize)]
struct FmlV12 {
    #[serde(rename = "Registries")]
    registries: RegistriesV12,
}

#[derive(Deserialize)]
struct RegistriesV12 {
    #[serde(rename = "minecraft:blocks", default)]
    blocks: Option<RegistryV12>,
}

#[derive(Deserialize, Default)]
struct RegistryV12 {
    #[serde(default)]
    ids: Vec<RegistryEntryV12>,
}

#[derive(Deserialize)]
struct RegistryEntryV12 {
    #[serde(rename = "K", default)]
    k: Option<String>,
    #[serde(rename = "V", default)]
    v: Option<i32>,
}

pub fn load_fml_registry_v12(path: &Path) -> Result<FmlRegistry12> {
    let nbt = read_gzipped(path)?;
    let data: LevelDatV12 = fastnbt::from_bytes(&nbt)
        .map_err(|e| format!("level.dat NBT parse: {}", e))?;

    let mut registry = FmlRegistry12::default();
    let Some(blocks) = data.fml.registries.blocks else {
        return Err(
            "level.dat has no FML.Registries.minecraft:blocks — is this a 1.12.2 \
             Forge world?"
                .into(),
        );
    };
    for entry in blocks.ids {
        let (Some(name), Some(id)) = (entry.k, entry.v) else {
            continue;
        };
        if name.is_empty() || id < 0 {
            continue;
        }
        let id = id as u32;
        if registry.blocks.insert(id, name.clone()).is_some() {
            debug!("Duplicate block id {} (replacing earlier entry)", id);
        }
    }
    Ok(registry)
}

// ----- shared --------------------------------------------------------------

/// Read and gzip-decode a level.dat. All pre-1.13 (and most 1.13+) level.dat
/// files are gzipped — chunk regions use zlib, level.dat uses gzip.
fn read_gzipped(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut dec = GzDecoder::new(&bytes[..]);
    let mut out = Vec::with_capacity(bytes.len() * 4);
    dec.read_to_end(&mut out)?;
    Ok(out)
}
