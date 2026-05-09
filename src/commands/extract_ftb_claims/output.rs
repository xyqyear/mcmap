// JSON output schema for `extract-ftb-claims`.
//
// One unified shape across all four FTB format families. `claims` are nested
// under their owning team; teams with zero claims are omitted (overlay
// rendering doesn't need them). `dimensions[]` is the ID→folder lookup table
// — an entry per dim id seen in claim data, with `exists` set when
// `<world>/<folder>/region/` is present on disk.

use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Debug)]
pub struct Output {
    pub mcmap_extract_ftb_claims_version: u32,
    pub detected_format: &'static str,
    pub world_dir: String,
    pub dimensions: Vec<DimensionEntry>,
    pub teams: Vec<Team>,
}

#[derive(Serialize, Debug)]
pub struct DimensionEntry {
    /// Raw FTB dim id. ResourceLocation string for SNBT family
    /// (`"minecraft:overworld"`); decimal int for pre-1.13 families (`"0"`,
    /// `"-1"`, `"7"`).
    pub id: String,
    /// Path relative to `world_dir`. `"."` for the overworld.
    pub folder: String,
    /// Whether `<world_dir>/<folder>/region/` exists on disk.
    pub exists: bool,
}

#[derive(Serialize, Debug)]
pub struct Team {
    /// For SNBT: the team UUID (dashed). For pre-1.13 families: the team-id
    /// string (typically lowercase username for player-owned teams) for
    /// `per_team_nbt`/`universe_dat`, or owner UUID for `latmod_json`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub team_type: TeamType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Member>,
    pub members: Vec<Member>,
    pub claims: Vec<Claim>,
}

#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TeamType {
    Player,
    Party,
    Server,
    Unknown,
}

#[derive(Serialize, Debug, Clone)]
pub struct Member {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Claim {
    /// Joins to `DimensionEntry.id`.
    pub dim: String,
    pub cx: i32,
    pub cz: i32,
    pub force_loaded: bool,
}
