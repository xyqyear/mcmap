// Family A0 parser — 1.7.10 upstream FTBU `LatMod/ClaimedChunks.json`.
//
// Schema: `{"<dim_int>": {"<32-char-uuid-hex-no-dashes>": [[cx, cz] |
// [cx, cz, force_loaded_byte], ...]}}`. No team concept — claims are owned
// per-player. Each owner becomes a synthesized solo player team in the
// output so the schema stays uniform across families.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use super::dim::{folder_to_relative, resolve_legacy};
use super::output::{Claim, DimensionEntry, Member, Output, SCHEMA_VERSION, Team, TeamType};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum LatmodChunkRecord {
    XzForce([i32; 3]),
    Xz([i32; 2]),
}

pub fn run(world_dir: &Path) -> Result<Output> {
    let path = world_dir.join("LatMod").join("ClaimedChunks.json");
    let bytes = fs::read(&path)?;
    let raw: HashMap<String, HashMap<String, Vec<LatmodChunkRecord>>> =
        serde_json::from_slice(&bytes)?;

    let mut all_dims: BTreeMap<i32, ()> = BTreeMap::new();
    let mut by_owner: HashMap<String, Vec<Claim>> = HashMap::new();

    for (dim_str, owners) in raw {
        let dim: i32 = match dim_str.parse() {
            Ok(d) => d,
            Err(_) => continue,
        };
        all_dims.insert(dim, ());
        for (uuid_hex, recs) in owners {
            let uuid = hex_to_uuid_string(&uuid_hex).unwrap_or(uuid_hex);
            for r in recs {
                let (cx, cz, force_loaded) = match r {
                    LatmodChunkRecord::Xz([x, z]) => (x, z, false),
                    LatmodChunkRecord::XzForce([x, z, f]) => (x, z, f != 0),
                };
                by_owner.entry(uuid.clone()).or_default().push(Claim {
                    dim: dim.to_string(),
                    cx,
                    cz,
                    force_loaded,
                });
            }
        }
    }

    let mut teams = Vec::new();
    let mut sorted_owners: Vec<(String, Vec<Claim>)> = by_owner.into_iter().collect();
    sorted_owners.sort_by(|a, b| a.0.cmp(&b.0));
    for (uuid, claims) in sorted_owners {
        let owner = Member {
            uuid: Some(uuid.clone()),
            name: None,
            rank: Some("owner".into()),
        };
        teams.push(Team {
            id: uuid,
            name: None,
            team_type: TeamType::Player,
            owner: Some(owner.clone()),
            members: vec![owner],
            claims,
        });
    }

    let dimensions: Vec<DimensionEntry> = all_dims
        .keys()
        .map(|&dim| {
            let (folder, exists) = resolve_legacy(world_dir, dim);
            DimensionEntry {
                id: dim.to_string(),
                folder: folder_to_relative(world_dir, &folder),
                exists,
            }
        })
        .collect();

    Ok(Output {
        mcmap_extract_ftb_claims_version: SCHEMA_VERSION,
        detected_format: "latmod_json",
        world_dir: world_dir.to_string_lossy().to_string(),
        dimensions,
        teams,
    })
}

fn hex_to_uuid_string(hex: &str) -> Option<String> {
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_uuid_canonical() {
        assert_eq!(
            hex_to_uuid_string("069a79f444e94726a5befca90e38aaf5").as_deref(),
            Some("069a79f4-44e9-4726-a5be-fca90e38aaf5")
        );
    }

    #[test]
    fn hex_to_uuid_rejects_bad() {
        assert_eq!(hex_to_uuid_string("short"), None);
        assert_eq!(hex_to_uuid_string("zz9a79f444e94726a5befca90e38aaf5"), None);
    }
}
