// Byte-level chunk removal from a region file.
//
// Empties the named region-relative chunk slots in a target `.mca`. For slots
// previously stored externally, the companion `c.<absX>.<absZ>.mcc` file is
// also deleted. Slots not listed are preserved verbatim. The shared region I/O
// (read/emit/atomic-write/.mcc bookkeeping) lives in `super::region_io`.

use clap::Args;
use serde::Serialize;
use std::path::PathBuf;

use super::region_io::{SlotState, apply_slot_mutations, parse_chunks, slot_index};
use crate::output::{emit, is_json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct RemoveChunksArgs {
    /// Path to the target r.X.Z.mca file (modified in place).
    #[arg(short, long)]
    target: PathBuf,

    /// Semicolon-separated list of region-relative chunk coords:
    /// `x,z;x,z;...`, each value in [0, 31].
    #[arg(short, long)]
    chunks: String,
}

#[derive(Serialize)]
struct ChunkRemoved<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    x: u8,
    z: u8,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    removed: usize,
}

pub fn execute(args: RemoveChunksArgs) -> Result<()> {
    let chunks = parse_chunks(&args.chunks)?;

    if !args.target.exists() {
        return Err(format!("target file does not exist: {}", args.target.display()).into());
    }

    let mutations: Vec<(usize, SlotState)> = chunks
        .iter()
        .map(|&(x, z)| (slot_index(x, z), SlotState::Empty))
        .collect();

    apply_slot_mutations(&args.target, &mutations)?;

    if is_json() {
        for &(x, z) in &chunks {
            emit(&ChunkRemoved {
                ty: "chunk_removed",
                x,
                z,
            });
        }
        emit(&ResultEvent {
            ty: "result",
            removed: chunks.len(),
        });
    }

    Ok(())
}
