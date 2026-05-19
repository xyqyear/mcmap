// `extract-players` — read vanilla player files from a world directory and
// emit current position plus a shared dimension lookup table.

mod coordinates;
mod discover;
mod output;

use clap::Args;
use log::info;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::commands::dim::{DimensionEntry, entry_for_id};
use crate::output::{emit_if_json, is_json};

use output::{Output, PlayerRecord, SCHEMA_VERSION, SkippedFile};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct ExtractPlayersArgs {
    /// World directory (the dir containing `level.dat` and playerdata/).
    #[arg(short, long)]
    pub world: PathBuf,

    /// Output path for the JSON file. Default: stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    players: usize,
    skipped: usize,
    dimensions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    data: &'a Output,
}

pub fn execute(args: ExtractPlayersArgs) -> Result<()> {
    if !args.world.is_dir() {
        return Err(format!("world directory not found: {}", args.world.display()).into());
    }

    let discovered = discover::discover(&args.world)?;
    let mut players = Vec::new();
    let mut skipped = Vec::new();
    let mut dim_ids: BTreeMap<String, ()> = BTreeMap::new();

    for file in discovered {
        match coordinates::extract(&file.path) {
            Ok(coords) => {
                dim_ids.insert(coords.dim.clone(), ());
                players.push(PlayerRecord {
                    id: file.id,
                    id_kind: file.id_kind,
                    source: file.source,
                    storage: file.storage,
                    data_version: coords.data_version,
                    dim: coords.dim,
                    pos: coords.pos,
                });
            }
            Err(err) => {
                skipped.push(SkippedFile {
                    source: file.source,
                    storage: file.storage,
                    reason: err.reason,
                    message: err.message,
                });
            }
        }
    }

    let dimensions: Vec<DimensionEntry> = dim_ids
        .keys()
        .map(|id| entry_for_id(&args.world, id))
        .collect();

    let out = Output {
        mcmap_extract_players_version: SCHEMA_VERSION,
        world_dir: args.world.to_string_lossy().to_string(),
        dimensions,
        players,
        skipped,
    };

    if let Some(path) = args.output.as_ref() {
        let json = serde_json::to_string_pretty(&out)?;
        std::fs::write(path, &json)?;
        info!("Wrote player data to {}", path.display());
    } else if !is_json() {
        let json = serde_json::to_string_pretty(&out)?;
        println!("{}", json);
    }

    info!(
        "Extraction complete: {} players, {} skipped, {} dimensions",
        out.players.len(),
        out.skipped.len(),
        out.dimensions.len()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        players: out.players.len(),
        skipped: out.skipped.len(),
        dimensions: out.dimensions.len(),
        output: args.output.as_ref().map(|p| p.display().to_string()),
        data: &out,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde::Serialize;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    #[derive(Serialize)]
    struct PlayerDat<D: Serialize> {
        #[serde(rename = "Pos")]
        pos: Vec<f64>,
        #[serde(rename = "Dimension")]
        dimension: D,
        #[serde(rename = "DataVersion", skip_serializing_if = "Option::is_none")]
        data_version: Option<i32>,
    }

    #[derive(Serialize)]
    struct SidecarDat {
        #[serde(rename = "Owner")]
        owner: String,
    }

    fn tmpdir() -> PathBuf {
        let id = format!(
            "mcmap_extract_players_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let p = std::env::temp_dir().join(id);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_gzip_nbt<T: Serialize>(path: &Path, value: &T) {
        let bytes = fastnbt::to_bytes(value).unwrap();
        let file = fs::File::create(path).unwrap();
        let mut enc = GzEncoder::new(file, Compression::fast());
        enc.write_all(&bytes).unwrap();
        enc.finish().unwrap();
    }

    #[test]
    fn execute_extracts_players_and_shared_dimensions() {
        let world = tmpdir();
        fs::create_dir_all(world.join("region")).unwrap();
        fs::create_dir_all(world.join("DIM7/region")).unwrap();
        fs::create_dir_all(world.join("playerdata")).unwrap();
        fs::create_dir_all(world.join("players/data")).unwrap();

        write_gzip_nbt(
            &world
                .join("playerdata")
                .join("11111111-2222-3333-4444-555555555555.dat"),
            &PlayerDat {
                pos: vec![1.0, 64.0, -2.5],
                dimension: 7,
                data_version: None,
            },
        );
        write_gzip_nbt(
            &world
                .join("players/data")
                .join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.dat"),
            &PlayerDat {
                pos: vec![10.0, 70.0, 20.0],
                dimension: "minecraft:overworld",
                data_version: Some(3955),
            },
        );
        write_gzip_nbt(
            &world
                .join("playerdata")
                .join("11111111-2222-3333-4444-555555555555_cyclic.dat"),
            &SidecarDat {
                owner: "11111111-2222-3333-4444-555555555555".into(),
            },
        );

        let output = world.join("players.json");
        execute(ExtractPlayersArgs {
            world: world.clone(),
            output: Some(output.clone()),
        })
        .unwrap();

        let json: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(json["players"].as_array().unwrap().len(), 2);
        assert_eq!(json["skipped"].as_array().unwrap().len(), 1);
        assert_eq!(json["skipped"][0]["reason"], "missing_pos");
        assert_eq!(json["dimensions"].as_array().unwrap().len(), 2);
        assert_eq!(json["dimensions"][0]["id"], "7");
        assert_eq!(json["dimensions"][0]["folder"], "DIM7");
        assert_eq!(json["dimensions"][0]["exists"], true);
        assert_eq!(json["dimensions"][1]["id"], "minecraft:overworld");
        assert_eq!(json["dimensions"][1]["folder"], ".");
        assert_eq!(json["dimensions"][1]["exists"], true);

        let _ = fs::remove_dir_all(&world);
    }
}
