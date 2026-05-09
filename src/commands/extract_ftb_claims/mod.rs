// `extract-ftb-claims` — read FTB chunk-claim data from a server world dir
// and emit a unified JSON document describing teams, members, claims, and
// the on-disk dim folder for each claimed dimension.
//
// Four format families are auto-detected (or selectable via `--format`):
//
//   - `snbt`         — 1.16+ FTB Chunks/Teams (plain SNBT)
//   - `per_team_nbt` — 1.7.10 GTNH ServerUtilities + 1.12.2 FTB Utilities
//                      (per-team gzipped NBT files)
//   - `universe_dat` — 1.10.2 FTB Utilities 3.x (single packed `universe.dat`)
//   - `latmod_json`  — 1.7.10 upstream FTBU (`LatMod/ClaimedChunks.json`)
//
// Output schema is in `output.rs`. Per-format research notes live under
// `D:\temp\ftb-claim-research\` (not committed; not required to build).

mod detect;
mod dim;
mod latmod_json;
mod output;
mod per_team_nbt;
mod snbt;
mod snbt_parser;
mod universe_dat;
mod uuid_util;

use clap::{Args, ValueEnum};
use log::info;
use serde::Serialize;
use std::path::PathBuf;

use crate::output::{emit_if_json, is_json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct ExtractFtbClaimsArgs {
    /// World directory (the dir containing `level.dat` and `region/`).
    #[arg(short, long)]
    pub world: PathBuf,

    /// Override format auto-detection. Useful when multiple FTB layouts
    /// coexist in the same world (e.g. legacy `LatMod/` alongside modern
    /// `serverutilities/`).
    #[arg(long, value_enum, default_value_t = FormatArg::Auto)]
    pub format: FormatArg,

    /// Output path for the JSON file. Default: stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum FormatArg {
    Auto,
    Snbt,
    PerTeamNbt,
    UniverseDat,
    LatmodJson,
}

impl FormatArg {
    fn to_format(self) -> Option<detect::Format> {
        match self {
            FormatArg::Auto => None,
            FormatArg::Snbt => Some(detect::Format::Snbt),
            FormatArg::PerTeamNbt => Some(detect::Format::PerTeamNbt),
            FormatArg::UniverseDat => Some(detect::Format::UniverseDat),
            FormatArg::LatmodJson => Some(detect::Format::LatmodJson),
        }
    }
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    detected_format: &'a str,
    teams: usize,
    claims: usize,
    dimensions: usize,
    /// Output file path when `--output` was passed; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// Full extraction payload. Embedded inline so a `--json` consumer
    /// can read the data without also reading a separate file.
    data: &'a output::Output,
}

pub fn execute(args: ExtractFtbClaimsArgs) -> Result<()> {
    if !args.world.is_dir() {
        return Err(format!("world directory not found: {}", args.world.display()).into());
    }
    let format = match args.format.to_format() {
        Some(f) => f,
        None => detect::detect(&args.world)
            .ok_or("could not detect FTB claim format in world directory")?,
    };
    info!("Using format: {}", format.label());

    let out = match format {
        detect::Format::Snbt => snbt::run(&args.world)?,
        detect::Format::PerTeamNbt => per_team_nbt::run(&args.world)?,
        detect::Format::UniverseDat => universe_dat::run(&args.world)?,
        detect::Format::LatmodJson => latmod_json::run(&args.world)?,
    };

    let claims_total: usize = out.teams.iter().map(|t| t.claims.len()).sum();
    let teams_total = out.teams.len();
    let dims_total = out.dimensions.len();

    if let Some(path) = args.output.as_ref() {
        let json = serde_json::to_string_pretty(&out)?;
        std::fs::write(path, &json)?;
        info!("Wrote claim data to {}", path.display());
    } else if !is_json() {
        // Human mode, no `--output`: print pretty JSON to stdout. In
        // `--json` mode we never print the pretty form here because that
        // would interleave with the NDJSON event stream.
        let json = serde_json::to_string_pretty(&out)?;
        println!("{}", json);
    }

    info!(
        "Extraction complete: {} teams, {} claims, {} dimensions",
        teams_total, claims_total, dims_total
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        detected_format: format.label(),
        teams: teams_total,
        claims: claims_total,
        dimensions: dims_total,
        output: args.output.as_ref().map(|p| p.display().to_string()),
        data: &out,
    });
    Ok(())
}
