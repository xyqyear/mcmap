// Family A1 parser — per-team gzipped NBT.
//
// Covers two layouts that share the schema and only differ in base path:
//   - 1.7.10 GTNH ServerUtilities: `<world>/serverutilities/teams/`
//   - 1.12.2 FTB Utilities 5.x:    `<world>/data/ftb_lib/teams/`
//
// Members and team owners are stored by **username** in this family (a key
// difference from A2 and B), so we walk `players/<lowercase_name>.dat` once
// to build a username→UUID map. Members whose player file is missing get
// emitted with `uuid: null` (the user accepted "no UUID = no UUID, output
// what we have").

use flate2::read::GzDecoder;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::dim::{folder_to_relative, resolve_legacy};
use super::output::{Claim, DimensionEntry, Member, Output, SCHEMA_VERSION, Team, TeamType};
use super::uuid_util;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Debug, Default)]
struct ClaimsFile {
    #[serde(rename = "ClaimedChunks", default)]
    claimed_chunks: HashMap<String, Vec<ChunkRecord>>,
}

#[derive(Deserialize, Debug)]
struct ChunkRecord {
    x: i32,
    z: i32,
    #[serde(default)]
    loaded: i8,
}

#[derive(Deserialize, Debug, Default)]
struct TeamFile {
    #[serde(rename = "Type", default)]
    team_type: Option<String>,
    #[serde(rename = "Owner", default)]
    owner: Option<String>,
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "Players", default)]
    players: HashMap<String, String>,
}

#[derive(Deserialize, Debug, Default)]
struct PlayerFile {
    #[serde(rename = "Name", default)]
    name: Option<String>,
    #[serde(rename = "UUID", default)]
    uuid: Option<String>,
}

pub fn run(world_dir: &Path) -> Result<Output> {
    let base = pick_base(world_dir)?;
    let claims_dir = base.join("teams").join("claimedchunks");
    let teams_dir = base.join("teams");
    let players_dir = base.join("players");

    let username_to_uuid = load_username_map(&players_dir);
    let team_meta = load_all_team_meta(&teams_dir, &username_to_uuid);

    let mut teams = Vec::new();
    let mut all_dims: BTreeMap<i32, ()> = BTreeMap::new();
    for entry in fs::read_dir(&claims_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("dat") {
            continue;
        }
        let team_id = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };
        let bytes = fs::read(&path)?;
        let parsed: ClaimsFile = read_gzip_nbt(&bytes)?;
        let mut claims = Vec::new();
        for (dim_str, recs) in &parsed.claimed_chunks {
            let dim: i32 = match dim_str.parse() {
                Ok(d) => d,
                Err(_) => continue,
            };
            all_dims.insert(dim, ());
            for r in recs {
                claims.push(Claim {
                    dim: dim.to_string(),
                    cx: r.x,
                    cz: r.z,
                    force_loaded: r.loaded != 0,
                });
            }
        }
        if claims.is_empty() {
            continue;
        }
        let (name, team_type, members, owner) = match team_meta.get(&team_id) {
            Some(m) => (m.name.clone(), m.team_type, m.members.clone(), m.owner.clone()),
            None => (None, TeamType::Unknown, Vec::new(), None),
        };
        teams.push(Team {
            id: team_id,
            name,
            team_type,
            owner,
            members,
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
        detected_format: "per_team_nbt",
        world_dir: world_dir.to_string_lossy().to_string(),
        dimensions,
        teams,
    })
}

fn pick_base(world_dir: &Path) -> Result<PathBuf> {
    let su = world_dir.join("serverutilities");
    if su.join("teams").join("claimedchunks").is_dir() {
        return Ok(su);
    }
    let ftbl = world_dir.join("data").join("ftb_lib");
    if ftbl.join("teams").join("claimedchunks").is_dir() {
        return Ok(ftbl);
    }
    Err("no FTB Utilities/ServerUtilities data directory found".into())
}

fn read_gzip_nbt<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut dec = GzDecoder::new(bytes);
        let mut buf = Vec::with_capacity(bytes.len() * 4);
        dec.read_to_end(&mut buf)?;
        Ok(fastnbt::from_bytes(&buf)?)
    } else {
        Ok(fastnbt::from_bytes(bytes)?)
    }
}

fn load_username_map(players_dir: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let entries = match fs::read_dir(players_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("dat") {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let player: PlayerFile = match read_gzip_nbt(&bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let (Some(name), Some(uuid)) = (player.name, player.uuid) {
            out.insert(name.to_lowercase(), uuid_util::normalize(&uuid));
        }
    }
    out
}

struct LoadedTeam {
    name: Option<String>,
    team_type: TeamType,
    members: Vec<Member>,
    owner: Option<Member>,
}

fn load_all_team_meta(
    teams_dir: &Path,
    username_to_uuid: &HashMap<String, String>,
) -> HashMap<String, LoadedTeam> {
    let mut out = HashMap::new();
    let entries = match fs::read_dir(teams_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("dat") {
            continue;
        }
        // FTBLib's loader filters to files with exactly one `.` — skip
        // backups like `<id>.dat.bak` or `<id>.dat~`.
        let name_str = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name_str.matches('.').count() != 1 {
            continue;
        }
        let team_id = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let parsed: TeamFile = match read_gzip_nbt(&bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        out.insert(team_id, build_loaded_team(parsed, username_to_uuid));
    }
    out
}

fn build_loaded_team(
    parsed: TeamFile,
    username_to_uuid: &HashMap<String, String>,
) -> LoadedTeam {
    let team_type = match parsed.team_type.as_deref() {
        Some("player") => TeamType::Player,
        Some("server") | Some("server_no_save") => TeamType::Server,
        Some("party") => TeamType::Party,
        _ => TeamType::Unknown,
    };

    let mut members = Vec::new();
    let mut owner = None;
    if let Some(o) = &parsed.owner {
        if !o.is_empty() {
            let m = Member {
                uuid: username_to_uuid.get(&o.to_lowercase()).cloned(),
                name: Some(o.clone()),
                rank: Some("owner".into()),
            };
            owner = Some(m.clone());
            members.push(m);
        }
    }
    let mut sorted_players: Vec<(&String, &String)> = parsed.players.iter().collect();
    sorted_players.sort_by(|a, b| a.0.cmp(b.0));
    for (username, status) in sorted_players {
        if let Some(o) = &parsed.owner {
            if username.eq_ignore_ascii_case(o) {
                continue;
            }
        }
        members.push(Member {
            uuid: username_to_uuid.get(&username.to_lowercase()).cloned(),
            name: Some(username.clone()),
            rank: Some(status.clone()),
        });
    }

    LoadedTeam {
        name: parsed.title.filter(|s| !s.is_empty()),
        team_type,
        members,
        owner,
    }
}
