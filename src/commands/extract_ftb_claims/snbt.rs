// Family B parser — 1.16+ FTB Chunks + FTB Teams.
//
// Inputs:
//   - `<world>/ftbchunks/<team-uuid>.snbt` — claims (one file per team)
//   - `<world>/ftbteams/{player|party|server}/<team-uuid>.snbt` — team meta
//   - Early 1.16 alternative: `<world>/data/ftbchunks/...`
//
// We parse every SNBT file with our own parser (`snbt_parser`) into a
// dynamic `SnbtValue` tree, then walk the tree manually to extract the
// fields we care about. Going through a typed `serde::Deserialize` would be
// cleaner but the SNBT dialect varies enough between MC versions
// (string/int-array UUIDs, optional fields, byte-as-bool conventions) that
// the dynamic walker is easier to keep schema-tolerant.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::output::{Claim, Member, Output, SCHEMA_VERSION, Team, TeamType};
use super::snbt_parser::{SnbtValue, parse};
use crate::commands::dim::{DimensionEntry, entry_for_modern};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn run(world_dir: &Path) -> Result<Output> {
    let chunks_dir = pick_chunks_dir(world_dir)?;
    let teams_dir = world_dir.join("ftbteams");
    let team_meta = load_all_team_meta(&teams_dir);

    let mut teams = Vec::new();
    let mut all_dims: BTreeMap<String, ()> = BTreeMap::new();

    for entry in fs::read_dir(&chunks_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("snbt") {
            continue;
        }
        let team_id = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };
        let bytes = fs::read(&path)?;
        let s = std::str::from_utf8(&bytes)?;
        let parsed = parse(s)?;
        let claims = extract_claims(&parsed, &mut all_dims);
        if claims.is_empty() {
            continue;
        }
        let (name, team_type, members, owner) = match team_meta.get(&team_id) {
            Some(m) => (
                m.name.clone(),
                m.team_type,
                m.members.clone(),
                m.owner.clone(),
            ),
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
        .map(|id| entry_for_modern(world_dir, id))
        .collect();

    Ok(Output {
        mcmap_extract_ftb_claims_version: SCHEMA_VERSION,
        detected_format: "snbt",
        world_dir: world_dir.to_string_lossy().to_string(),
        dimensions,
        teams,
    })
}

fn pick_chunks_dir(world_dir: &Path) -> Result<PathBuf> {
    let primary = world_dir.join("ftbchunks");
    if primary.is_dir() {
        return Ok(primary);
    }
    let legacy = world_dir.join("data").join("ftbchunks");
    if legacy.is_dir() {
        return Ok(legacy);
    }
    Err("no ftbchunks/ directory found in world".into())
}

fn extract_claims(root: &SnbtValue, all_dims: &mut BTreeMap<String, ()>) -> Vec<Claim> {
    let mut claims = Vec::new();
    let chunks = match root.as_compound().and_then(|m| m.get("chunks")) {
        Some(c) => c,
        None => return claims,
    };
    let chunks_map = match chunks.as_compound() {
        Some(m) => m,
        None => return claims,
    };
    for (dim, list_val) in chunks_map {
        let list = match list_val.as_list() {
            Some(l) => l,
            None => continue,
        };
        for entry in list {
            let m = match entry.as_compound() {
                Some(m) => m,
                None => continue,
            };
            let cx = match m.get("x").and_then(|v| v.as_i32()) {
                Some(v) => v,
                None => continue,
            };
            let cz = match m.get("z").and_then(|v| v.as_i32()) {
                Some(v) => v,
                None => continue,
            };
            let force_loaded = m
                .get("force_loaded")
                .and_then(|v| v.as_i64())
                .map(|n| n > 0)
                .unwrap_or(false);
            all_dims.insert(dim.clone(), ());
            claims.push(Claim {
                dim: dim.clone(),
                cx,
                cz,
                force_loaded,
            });
        }
    }
    claims
}

struct LoadedTeam {
    name: Option<String>,
    team_type: TeamType,
    members: Vec<Member>,
    owner: Option<Member>,
}

fn load_all_team_meta(teams_dir: &Path) -> std::collections::HashMap<String, LoadedTeam> {
    let mut out = std::collections::HashMap::new();
    for sub in &["player", "party", "server"] {
        let dir = teams_dir.join(sub);
        if !dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("snbt") {
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
            let s = match std::str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let parsed = match parse(s) {
                Ok(p) => p,
                Err(_) => continue,
            };
            out.insert(team_id.clone(), build_loaded_team(&team_id, sub, &parsed));
        }
    }
    out
}

fn build_loaded_team(team_id: &str, sub: &str, parsed: &SnbtValue) -> LoadedTeam {
    let m = match parsed.as_compound() {
        Some(m) => m,
        None => {
            return LoadedTeam {
                name: None,
                team_type: TeamType::Unknown,
                members: Vec::new(),
                owner: None,
            };
        }
    };

    let team_type = m
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| sub.to_string());
    let team_type = match team_type.as_str() {
        "player" => TeamType::Player,
        "party" => TeamType::Party,
        "server" => TeamType::Server,
        _ => TeamType::Unknown,
    };

    let name = m
        .get("properties")
        .and_then(|v| v.as_compound())
        .and_then(|p| p.get("ftbteams:display_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let player_name = m
        .get("player_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut members = Vec::new();
    let mut owner = None;
    if let Some(ranks) = m.get("ranks").and_then(|v| v.as_compound()) {
        let mut sorted: Vec<(&String, &SnbtValue)> = ranks.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (uuid, rank_val) in sorted {
            let rank = rank_val.as_str().unwrap_or("").to_string();
            let display_name = if matches!(team_type, TeamType::Player) && uuid == team_id {
                player_name.clone()
            } else {
                None
            };
            let member = Member {
                uuid: Some(uuid.clone()),
                name: display_name,
                rank: Some(rank.clone()),
            };
            if rank == "owner" && owner.is_none() {
                owner = Some(member.clone());
            }
            members.push(member);
        }
    }

    LoadedTeam {
        name,
        team_type,
        members,
        owner,
    }
}
