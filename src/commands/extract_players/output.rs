use crate::commands::dim::DimensionEntry;
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Debug)]
pub struct Output {
    pub mcmap_extract_players_version: u32,
    pub world_dir: String,
    pub dimensions: Vec<DimensionEntry>,
    pub players: Vec<PlayerRecord>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Serialize, Debug)]
pub struct PlayerRecord {
    pub id: String,
    pub id_kind: PlayerIdKind,
    /// Path relative to `world_dir`, always with `/` separators.
    pub source: String,
    pub storage: StorageKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_version: Option<i32>,
    /// Joins to `DimensionEntry.id`.
    pub dim: String,
    pub pos: Position,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Serialize, Debug)]
pub struct SkippedFile {
    /// Path relative to `world_dir`, always with `/` separators.
    pub source: String,
    pub storage: StorageKind,
    pub reason: SkipReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    Playerdata,
    PlayersData,
    LegacyPlayers,
}

#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayerIdKind {
    Uuid,
    Name,
}

#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    ParseError,
    MissingPos,
    InvalidPos,
    MissingDimension,
    InvalidDimension,
}
