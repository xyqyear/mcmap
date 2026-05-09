// Auto-detect which FTB claim format a world directory uses. Probed in
// most-current-to-least-current order so a world that migrated from FTBU
// 1.7.10 → ServerUtilities (and still has the legacy `LatMod/` directory
// alongside the modern `serverutilities/`) returns the modern format.

use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// 1.16+ FTB Chunks/Teams (plain SNBT).
    Snbt,
    /// 1.7.10 GTNH ServerUtilities + 1.12.2 FTB Utilities 5.x (per-team
    /// gzipped NBT files).
    PerTeamNbt,
    /// 1.10.2 FTB Utilities 3.x (single `universe.dat` with packed int
    /// arrays).
    UniverseDat,
    /// 1.7.10 upstream FTBU (LatvianModder) — `LatMod/ClaimedChunks.json`.
    LatmodJson,
}

impl Format {
    pub fn label(&self) -> &'static str {
        match self {
            Format::Snbt => "snbt",
            Format::PerTeamNbt => "per_team_nbt",
            Format::UniverseDat => "universe_dat",
            Format::LatmodJson => "latmod_json",
        }
    }
}

pub fn detect(world_dir: &Path) -> Option<Format> {
    if world_dir.join("ftbchunks").is_dir()
        || world_dir.join("ftbteams").is_dir()
        || world_dir.join("data").join("ftbchunks").is_dir()
    {
        return Some(Format::Snbt);
    }
    if world_dir
        .join("serverutilities")
        .join("teams")
        .join("claimedchunks")
        .is_dir()
        || world_dir
            .join("data")
            .join("ftb_lib")
            .join("teams")
            .join("claimedchunks")
            .is_dir()
    {
        return Some(Format::PerTeamNbt);
    }
    if world_dir
        .join("data")
        .join("ftb_lib")
        .join("universe.dat")
        .is_file()
    {
        return Some(Format::UniverseDat);
    }
    if world_dir
        .join("LatMod")
        .join("ClaimedChunks.json")
        .is_file()
    {
        return Some(Format::LatmodJson);
    }
    None
}
