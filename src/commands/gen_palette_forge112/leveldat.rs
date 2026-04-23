// Parse `level.dat` to pull the modern FML block ID registry out (1.8+).
//
// Forge rewrote registries in 1.8: instead of one flat `FML.ItemData` list
// mixing blocks + items (with `\x01` / `\x02` byte prefixes — see the 1.7.10
// parser at `gen_palette_legacy::leveldat`), 1.12.2 worlds use:
//
//   FML.Registries.<registry_name>.ids: List<{K: "ns:name", V: int_id}>
//
// Block ids live under `minecraft:blocks`. The id range is uncapped under
// REI/JEID (which lift the vanilla 4095 ceiling), so we treat `V` as a full
// `i32` — no masking — and cast to `u32` for downstream lookups.
//
// We deliberately don't read the other registries (`items`, `potions`,
// `biomes`, `enchantments`, etc.) — only blocks affect a top-down map.

use flate2::read::GzDecoder;
use log::debug;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::Result;

#[derive(Default)]
pub struct FmlRegistry {
    /// Numeric block id → `"namespace:name"`. Range is `0 .. ~28-bit` under
    /// REI; in practice modpacks stay under ~10000.
    pub blocks: HashMap<u32, String>,
}

#[derive(Deserialize)]
struct LevelDat {
    #[serde(rename = "FML")]
    fml: Fml,
}

#[derive(Deserialize)]
struct Fml {
    #[serde(rename = "Registries")]
    registries: Registries,
}

#[derive(Deserialize)]
struct Registries {
    #[serde(rename = "minecraft:blocks", default)]
    blocks: Option<Registry>,
}

#[derive(Deserialize, Default)]
struct Registry {
    #[serde(default)]
    ids: Vec<RegistryEntry>,
}

#[derive(Deserialize)]
struct RegistryEntry {
    #[serde(rename = "K", default)]
    k: Option<String>,
    #[serde(rename = "V", default)]
    v: Option<i32>,
}

/// Read and gzip-decode a level.dat file. 1.12.2 worlds use gzip for
/// level.dat the same way 1.7.10 does (only chunk regions use zlib).
fn read_gzipped(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut dec = GzDecoder::new(&bytes[..]);
    let mut out = Vec::with_capacity(bytes.len() * 4);
    dec.read_to_end(&mut out)?;
    Ok(out)
}

pub fn load_fml_registry(path: &Path) -> Result<FmlRegistry> {
    let nbt = read_gzipped(path)?;
    let data: LevelDat = fastnbt::from_bytes(&nbt)
        .map_err(|e| format!("level.dat NBT parse: {}", e))?;

    let mut registry = FmlRegistry::default();
    let Some(blocks) = data.fml.registries.blocks else {
        return Err(
            "level.dat has no FML.Registries.minecraft:blocks — is this a 1.12.2 \
             Forge world? (1.7.10 worlds use FML.ItemData; pass --legacy instead)"
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
