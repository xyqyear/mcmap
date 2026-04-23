// Parse `level.dat` to pull the FML block ID registry out.
//
// Pre-Forge-rewrite format (1.7.10 uses this): `FML.ItemData` is a
// `TAG_List<Compound>` where each entry has `K` (prefixed name) and `V`
// (numeric id). Prefix `\x01` = block, `\x02` = item. Some entries have
// empty or malformed K strings — those are skipped silently.

use flate2::read::GzDecoder;
use log::debug;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::Result;

#[derive(Default)]
pub struct FmlRegistry {
    pub blocks: HashMap<u16, String>,
    pub items: HashMap<u16, String>,
}

#[derive(Deserialize)]
struct LevelDat {
    #[serde(rename = "FML")]
    fml: Fml,
}

#[derive(Deserialize)]
struct Fml {
    #[serde(rename = "ItemData", default)]
    item_data: Vec<ItemEntry>,
}

#[derive(Deserialize)]
struct ItemEntry {
    #[serde(rename = "K", default)]
    k: Option<String>,
    #[serde(rename = "V", default)]
    v: Option<i32>,
}

/// Read and gzip-decode a level.dat file. 1.7.10 worlds always use gzip for
/// level.dat (unlike regions, which use zlib).
fn read_gzipped(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut dec = GzDecoder::new(&bytes[..]);
    let mut out = Vec::with_capacity(bytes.len() * 4);
    dec.read_to_end(&mut out)?;
    Ok(out)
}

pub fn load_fml_registry(path: &Path) -> Result<FmlRegistry> {
    let nbt = read_gzipped(path)?;
    let data: LevelDat =
        fastnbt::from_bytes(&nbt).map_err(|e| format!("level.dat NBT parse: {}", e))?;

    let mut registry = FmlRegistry::default();
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
