// Family A2 parser — 1.10.2 FTB Utilities 3.x `universe.dat`.
//
// All claims live in a single file at `<world>/data/ftb_lib/universe.dat`,
// keyed by **owning player UUID** (dashed string) under
// `Data["ftbu:data"]["Chunks"]`. Each chunk is an int-array of length 3
// (`[dim, cx, cz]`) or 4 (`[dim, cx, cz, flags]`). Bit 0 of `flags` is the
// force-load bit.
//
// Team membership is in sibling files at `data/ftb_lib/teams/` and player
// info at `data/ftb_lib/players/`. Per-chunk owner is a player UUID, not a
// team — to roll up by team we look up each player's `TeamID` field. Players
// without a team (`TeamID = ""`) get a synthesized solo team.

use fastnbt::Value;
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;

use super::dim::{folder_to_relative, resolve_legacy};
use super::output::{Claim, DimensionEntry, Member, Output, SCHEMA_VERSION, Team, TeamType};
use super::uuid_util;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const FORCE_LOAD_BIT: i32 = 1 << 0;

#[derive(Deserialize, Debug, Default)]
struct UniverseDat {
    #[serde(rename = "Data", default)]
    data: HashMap<String, Value>,
}

#[derive(Deserialize, Debug, Default)]
struct PlayerFile {
    #[serde(rename = "Name", default)]
    name: Option<String>,
    #[serde(rename = "TeamID", default)]
    team_id: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct TeamFile {
    #[serde(rename = "Owner", default)]
    owner: Option<String>,
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "Players", default)]
    players: HashMap<String, String>,
}

pub fn run(world_dir: &Path) -> Result<Output> {
    let base = world_dir.join("data").join("ftb_lib");
    let universe_path = base.join("universe.dat");
    let teams_dir = base.join("teams");
    let players_dir = base.join("players");

    let bytes = fs::read(&universe_path)?;
    let universe: UniverseDat = read_gzip_nbt(&bytes)?;
    let ftbu_data = match universe.data.get("ftbu:data") {
        Some(Value::Compound(m)) => m,
        _ => return Ok(empty_output(world_dir)),
    };
    let chunks = match ftbu_data.get("Chunks") {
        Some(Value::Compound(c)) => c,
        _ => return Ok(empty_output(world_dir)),
    };

    let mut by_owner: HashMap<String, Vec<Claim>> = HashMap::new();
    let mut all_dims: BTreeMap<i32, ()> = BTreeMap::new();
    for (uuid, val) in chunks {
        let owner = uuid_util::normalize(uuid);
        let list = match val {
            Value::List(l) => l,
            _ => continue,
        };
        for entry in list {
            let arr: &[i32] = match entry {
                Value::IntArray(a) => a,
                _ => continue,
            };
            if arr.len() < 3 {
                continue;
            }
            let dim = arr[0];
            let cx = arr[1];
            let cz = arr[2];
            let flags = if arr.len() >= 4 { arr[3] } else { 0 };
            let force_loaded = (flags & FORCE_LOAD_BIT) != 0;
            all_dims.insert(dim, ());
            by_owner.entry(owner.clone()).or_default().push(Claim {
                dim: dim.to_string(),
                cx,
                cz,
                force_loaded,
            });
        }
    }

    let player_info = load_player_info(&players_dir);
    let team_info = load_team_info(&teams_dir, &player_info);

    let mut teams: Vec<Team> = Vec::new();
    let mut team_claims: HashMap<String, Vec<Claim>> = HashMap::new();
    let mut orphan: HashMap<String, Vec<Claim>> = HashMap::new();
    for (owner_uuid, claims) in by_owner {
        let team_id = player_info
            .get(&owner_uuid)
            .and_then(|p| p.team_id.clone())
            .filter(|s| !s.is_empty());
        match team_id {
            Some(tid) => team_claims.entry(tid).or_default().extend(claims),
            None => {
                orphan.insert(owner_uuid, claims);
            }
        }
    }

    for (team_id, claims) in team_claims {
        let info = team_info.get(&team_id);
        let (name, team_type, members, owner) = match info {
            Some(t) => (
                t.name.clone(),
                t.team_type,
                t.members.clone(),
                t.owner.clone(),
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
    for (uuid, claims) in orphan {
        let name = player_info.get(&uuid).and_then(|p| p.name.clone());
        let owner = Member {
            uuid: Some(uuid.clone()),
            name: name.clone(),
            rank: Some("owner".into()),
        };
        teams.push(Team {
            id: uuid,
            name,
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
        detected_format: "universe_dat",
        world_dir: world_dir.to_string_lossy().to_string(),
        dimensions,
        teams,
    })
}

fn empty_output(world_dir: &Path) -> Output {
    Output {
        mcmap_extract_ftb_claims_version: SCHEMA_VERSION,
        detected_format: "universe_dat",
        world_dir: world_dir.to_string_lossy().to_string(),
        dimensions: vec![],
        teams: vec![],
    }
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

fn load_player_info(players_dir: &Path) -> HashMap<String, PlayerFile> {
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
        let uuid = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let parsed: PlayerFile = match read_gzip_nbt(&bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        out.insert(uuid, parsed);
    }
    out
}

struct LoadedTeam {
    name: Option<String>,
    team_type: TeamType,
    members: Vec<Member>,
    owner: Option<Member>,
}

fn load_team_info(
    teams_dir: &Path,
    player_info: &HashMap<String, PlayerFile>,
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

        let mut members = Vec::new();
        let mut owner = None;
        if let Some(o) = &parsed.owner {
            if !o.is_empty() {
                let normalized = uuid_util::normalize(o);
                let m = Member {
                    uuid: Some(normalized),
                    name: player_info.get(o).and_then(|p| p.name.clone()),
                    rank: Some("owner".into()),
                };
                owner = Some(m.clone());
                members.push(m);
            }
        }
        let mut sorted: Vec<(&String, &String)> = parsed.players.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (member_uuid, status) in sorted {
            if Some(member_uuid) == parsed.owner.as_ref() {
                continue;
            }
            members.push(Member {
                uuid: Some(uuid_util::normalize(member_uuid)),
                name: player_info.get(member_uuid).and_then(|p| p.name.clone()),
                rank: Some(status.clone()),
            });
        }
        out.insert(
            team_id,
            LoadedTeam {
                name: parsed.title.filter(|s| !s.is_empty()),
                team_type: TeamType::Player,
                members,
                owner,
            },
        );
    }
    out
}
