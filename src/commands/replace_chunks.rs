// Byte-level chunk replacement between region files.
//
// Copies named region-relative chunk slots from a source `.mca` into a target
// `.mca` without decoding NBT. Honors the 1.15+ `.mcc` external-chunk overflow
// mechanism. Heavy lifting (read/emit/atomic-write/.mcc bookkeeping) lives in
// `super::region_io`; this file is the CLI surface.
//
// See `docs/replace_chunks.md` for the full spec.

use clap::Args;
use serde::Serialize;
use std::path::{Path, PathBuf};

use super::region_io::{
    SlotState, apply_slot_mutations, is_placeholder_region, parse_chunks, read_slot,
    region_coords, slot_index,
};
use crate::output::{emit, is_json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct ReplaceChunksArgs {
    /// Path to the source r.X.Z.mca file (read-only).
    #[arg(short, long)]
    source: PathBuf,

    /// Path to the target r.X.Z.mca file (created or modified in place).
    #[arg(short, long)]
    target: PathBuf,

    /// Semicolon-separated list of region-relative chunk coords:
    /// `x,z;x,z;...`, each value in [0, 31].
    #[arg(short, long)]
    chunks: String,
}

#[derive(Serialize)]
struct ChunkReplaced<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    x: u8,
    z: u8,
    source_kind: &'a str,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    replaced: usize,
}

pub fn execute(args: ReplaceChunksArgs) -> Result<()> {
    let chunks = parse_chunks(&args.chunks)?;

    if args.source == args.target {
        return Err("source and target are the same path".into());
    }
    if let (Ok(a), Ok(b)) = (
        std::fs::canonicalize(&args.source),
        std::fs::canonicalize(&args.target),
    ) && a == b
    {
        return Err("source and target resolve to the same file".into());
    }

    if !args.source.exists() {
        return Err(format!("source file does not exist: {}", args.source.display()).into());
    }
    let source_bytes = std::fs::read(&args.source)?;
    let source_is_placeholder = is_placeholder_region(&source_bytes);
    let source_region = region_coords(&args.source);
    let source_dir = args.source.parent().unwrap_or(Path::new(""));

    let mut mutations: Vec<(usize, SlotState)> = Vec::with_capacity(chunks.len());
    let mut source_kinds: Vec<&'static str> = Vec::with_capacity(chunks.len());
    for &(x, z) in &chunks {
        let slot = slot_index(x, z);
        let state = if source_is_placeholder {
            SlotState::Empty
        } else {
            read_slot(&source_bytes, slot, source_dir, source_region, true, "source")?
        };
        let kind = match &state {
            SlotState::Empty => "empty",
            SlotState::Inline { .. } => "inline",
            SlotState::External { .. } => "external",
        };
        source_kinds.push(kind);
        mutations.push((slot, state));
    }

    apply_slot_mutations(&args.target, &mutations)?;

    if is_json() {
        for (i, &(x, z)) in chunks.iter().enumerate() {
            emit(&ChunkReplaced {
                ty: "chunk_replaced",
                x,
                z,
                source_kind: source_kinds[i],
            });
        }
        emit(&ResultEvent {
            ty: "result",
            replaced: chunks.len(),
        });
    }

    Ok(())
}
