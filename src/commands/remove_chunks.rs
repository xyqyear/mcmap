// Byte-level chunk removal from a region file.
//
// Empties the named region-relative chunk slots in a target `.mca`. For slots
// previously stored externally, the companion `c.<absX>.<absZ>.mcc` file is
// also deleted. Slots not listed are preserved verbatim. The shared region I/O
// (read/emit/atomic-write/.mcc bookkeeping) lives in `super::region_io`.

use clap::Args;
use serde::Serialize;
use std::path::PathBuf;

use super::region_io::{
    HEADER_SECTORS, SECTOR_BYTES, SlotState, apply_slot_mutations, parse_chunks, slot_index,
};
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    was_empty: bool,
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

    // A sub-header target (most often a 0-byte vanilla poi placeholder) already
    // has no chunks recorded — the requested removals are no-ops. Leave the
    // file alone rather than promoting it to an 8192-byte all-zero header.
    let target_was_empty =
        (std::fs::metadata(&args.target)?.len() as usize) < HEADER_SECTORS * SECTOR_BYTES;

    if !target_was_empty {
        let mutations: Vec<(usize, SlotState)> = chunks
            .iter()
            .map(|&(x, z)| (slot_index(x, z), SlotState::Empty))
            .collect();
        apply_slot_mutations(&args.target, &mutations)?;
    }

    if is_json() {
        for &(x, z) in &chunks {
            emit(&ChunkRemoved {
                ty: "chunk_removed",
                x,
                z,
                was_empty: target_was_empty,
            });
        }
        emit(&ResultEvent {
            ty: "result",
            removed: chunks.len(),
        });
    }

    Ok(())
}
