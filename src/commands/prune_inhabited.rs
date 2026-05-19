use clap::{Args, ValueEnum};
use flate2::read::{GzDecoder, ZlibDecoder};
use log::{info, warn};
use lz4_java_wrc::Lz4BlockInput;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use super::region_io::{
    SLOT_COUNT, SlotState, apply_slot_mutations, is_placeholder_region, read_slot, region_coords,
};
use super::util::parse_region_filename;
use crate::output::{emit, is_json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct PruneInhabitedArgs {
    /// Path to scan. May be a world dir, dimension dir, or any parent dir.
    #[arg(value_name = "PATH")]
    path: PathBuf,

    /// Delete chunks whose InhabitedTime is strictly less than this value.
    #[arg(short, long, value_name = "TICKS")]
    threshold: i64,

    /// Selection mode. `chunks` deletes per chunk; `regions` deletes a whole
    /// region only when every present chunk in that region is below threshold.
    #[arg(long, value_enum, default_value_t = PruneMode::Chunks)]
    mode: PruneMode,

    /// Print what would be deleted without modifying files.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PruneMode {
    Chunks,
    Regions,
}

#[derive(Clone, Debug)]
struct RegionDir {
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct RegionPlan {
    path: PathBuf,
    region_x: isize,
    region_z: isize,
    present_chunks: usize,
    prune_slots: Vec<ChunkPlan>,
}

#[derive(Clone, Debug)]
struct ChunkPlan {
    slot: usize,
    rel_x: u8,
    rel_z: u8,
    chunk_x: isize,
    chunk_z: isize,
    inhabited_time: i64,
}

#[derive(Deserialize)]
struct CurrentProbe {
    #[serde(rename = "InhabitedTime", default)]
    inhabited_time: Option<i64>,
}

#[derive(Deserialize)]
struct WrappedProbe {
    #[serde(rename = "Level")]
    level: LegacyProbe,
}

#[derive(Deserialize)]
struct LegacyProbe {
    #[serde(rename = "InhabitedTime", default)]
    inhabited_time: Option<i64>,
}

#[derive(Serialize)]
struct RegionDirEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    path: String,
    regions: usize,
}

#[derive(Serialize)]
struct ChunkEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    region: String,
    chunk_x: isize,
    chunk_z: isize,
    rel_x: u8,
    rel_z: u8,
    inhabited_time: i64,
    dry_run: bool,
}

#[derive(Serialize)]
struct RegionEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    region: String,
    region_x: isize,
    region_z: isize,
    chunks: usize,
    max_inhabited_time: i64,
    dry_run: bool,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    mode: &'a str,
    dry_run: bool,
    region_dirs: usize,
    regions_scanned: usize,
    chunks_scanned: usize,
    chunks_selected: usize,
    regions_selected: usize,
}

pub fn execute(args: PruneInhabitedArgs) -> Result<()> {
    if args.threshold < 0 {
        return Err("--threshold must be non-negative".into());
    }
    if !args.path.exists() {
        return Err(format!("path does not exist: {}", args.path.display()).into());
    }

    let region_dirs = discover_region_dirs(&args.path)?;
    if region_dirs.is_empty() {
        return Err(format!(
            "no region directories containing .mca files found under {}",
            args.path.display()
        )
        .into());
    }
    info!("Found {} region directories", region_dirs.len());

    let mut regions_scanned = 0usize;
    let mut chunks_scanned = 0usize;
    let mut chunks_selected = 0usize;
    let mut selected_region_count = 0usize;
    let mut all_plans = Vec::new();

    for dir in &region_dirs {
        let region_files = list_region_files(&dir.path)?;
        if is_json() {
            emit(&RegionDirEvent {
                ty: "region_dir",
                path: dir.path.display().to_string(),
                regions: region_files.len(),
            });
        }

        for region_path in region_files {
            match plan_region(&region_path, args.threshold, args.mode) {
                Ok(Some(plan)) => {
                    regions_scanned += 1;
                    chunks_scanned += plan.present_chunks;
                    chunks_selected += plan.prune_slots.len();
                    if args.mode == PruneMode::Regions && !plan.prune_slots.is_empty() {
                        selected_region_count += 1;
                    }
                    all_plans.push(plan);
                }
                Ok(None) => regions_scanned += 1,
                Err(e) => warn!("{}: {}", region_path.display(), e),
            }
        }
    }

    if args.mode == PruneMode::Chunks {
        selected_region_count = all_plans
            .iter()
            .filter(|p| !p.prune_slots.is_empty())
            .count();
    }

    for plan in &all_plans {
        if plan.prune_slots.is_empty() {
            continue;
        }

        if is_json() {
            match args.mode {
                PruneMode::Chunks => {
                    for chunk in &plan.prune_slots {
                        emit(&ChunkEvent {
                            ty: "chunk_pruned",
                            region: plan.path.display().to_string(),
                            chunk_x: chunk.chunk_x,
                            chunk_z: chunk.chunk_z,
                            rel_x: chunk.rel_x,
                            rel_z: chunk.rel_z,
                            inhabited_time: chunk.inhabited_time,
                            dry_run: args.dry_run,
                        });
                    }
                }
                PruneMode::Regions => {
                    let max_time = plan
                        .prune_slots
                        .iter()
                        .map(|c| c.inhabited_time)
                        .max()
                        .unwrap_or(0);
                    emit(&RegionEvent {
                        ty: "region_pruned",
                        region: plan.path.display().to_string(),
                        region_x: plan.region_x,
                        region_z: plan.region_z,
                        chunks: plan.prune_slots.len(),
                        max_inhabited_time: max_time,
                        dry_run: args.dry_run,
                    });
                }
            }
        } else {
            print_plan(plan, args.mode, args.dry_run);
        }

        if !args.dry_run {
            apply_plan(plan)?;
        }
    }

    if is_json() {
        emit(&ResultEvent {
            ty: "result",
            mode: match args.mode {
                PruneMode::Chunks => "chunks",
                PruneMode::Regions => "regions",
            },
            dry_run: args.dry_run,
            region_dirs: region_dirs.len(),
            regions_scanned,
            chunks_scanned,
            chunks_selected,
            regions_selected: selected_region_count,
        });
    }

    Ok(())
}

fn discover_region_dirs(root: &Path) -> Result<Vec<RegionDir>> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    if root.is_file() {
        return Ok(dirs);
    }
    walk_dirs(root, &mut |path| {
        if path.file_name().and_then(|s| s.to_str()) != Some("region") {
            return Ok(());
        }
        if contains_region_file(path)? {
            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if seen.insert(canonical) {
                dirs.push(RegionDir {
                    path: path.to_path_buf(),
                });
            }
        }
        Ok(())
    })?;
    dirs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(dirs)
}

fn walk_dirs(path: &Path, f: &mut impl FnMut(&Path) -> Result<()>) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    f(path)?;
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("failed to read directory {}: {}", path.display(), e);
            return Ok(());
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                warn!(
                    "failed to read directory entry under {}: {}",
                    path.display(),
                    e
                );
                continue;
            }
        };
        let child = entry.path();
        if child.is_dir() {
            walk_dirs(&child, f)?;
        }
    }
    Ok(())
}

fn contains_region_file(dir: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if entry.path().is_file() && parse_region_filename(name).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn list_region_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if parse_region_filename(name).is_some() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn plan_region(region_path: &Path, threshold: i64, mode: PruneMode) -> Result<Option<RegionPlan>> {
    let Some((rx, rz)) = region_coords(region_path) else {
        return Ok(None);
    };
    let bytes = std::fs::read(region_path)?;
    if is_placeholder_region(&bytes) {
        return Ok(None);
    }

    let mut present = Vec::new();
    let region_dir = region_path.parent().unwrap_or(Path::new(""));
    for slot in 0..SLOT_COUNT {
        let state = read_slot(&bytes, slot, region_dir, Some((rx, rz)), true, "scan")?;
        if matches!(state, SlotState::Empty) {
            continue;
        }
        let chunk_bytes = decompress_slot(&state)?;
        let inhabited_time = read_inhabited_time(&chunk_bytes)?;
        let rel_x = (slot % 32) as u8;
        let rel_z = (slot / 32) as u8;
        present.push(ChunkPlan {
            slot,
            rel_x,
            rel_z,
            chunk_x: rx * 32 + rel_x as isize,
            chunk_z: rz * 32 + rel_z as isize,
            inhabited_time,
        });
    }

    if present.is_empty() {
        return Ok(None);
    }

    let prune_slots = match mode {
        PruneMode::Chunks => present
            .iter()
            .filter(|c| c.inhabited_time < threshold)
            .cloned()
            .collect(),
        PruneMode::Regions => {
            if present.iter().all(|c| c.inhabited_time < threshold) {
                present.clone()
            } else {
                Vec::new()
            }
        }
    };

    Ok(Some(RegionPlan {
        path: region_path.to_path_buf(),
        region_x: rx,
        region_z: rz,
        present_chunks: present.len(),
        prune_slots,
    }))
}

fn decompress_slot(state: &SlotState) -> Result<Vec<u8>> {
    match state {
        SlotState::Empty => Err("cannot decompress empty slot".into()),
        SlotState::Inline {
            scheme, payload, ..
        } => decompress_payload(*scheme, payload),
        SlotState::External { scheme, mcc, .. } => {
            let payload = mcc
                .as_ref()
                .ok_or("external chunk slot has no loaded .mcc payload")?;
            decompress_payload(*scheme & 0x7F, payload)
        }
    }
}

fn decompress_payload(scheme: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match scheme {
        1 => {
            GzDecoder::new(payload).read_to_end(&mut out)?;
        }
        2 => {
            ZlibDecoder::new(payload).read_to_end(&mut out)?;
        }
        3 => out.extend_from_slice(payload),
        4 => {
            let mut decoder = Lz4BlockInput::new(Cursor::new(payload));
            decoder.read_to_end(&mut out)?;
        }
        127 => return Err("custom region-file compression is not supported".into()),
        other => {
            return Err(format!("unsupported region-file compression scheme {}", other).into());
        }
    };
    Ok(out)
}

fn read_inhabited_time(chunk_nbt: &[u8]) -> Result<i64> {
    if let Ok(current) = fastnbt::from_bytes::<CurrentProbe>(chunk_nbt) {
        if let Some(value) = current.inhabited_time {
            return Ok(value);
        }
    }
    let wrapped: WrappedProbe = fastnbt::from_bytes(chunk_nbt)?;
    Ok(wrapped.level.inhabited_time.unwrap_or(0))
}

fn apply_plan(plan: &RegionPlan) -> Result<()> {
    let mut by_path: BTreeMap<PathBuf, BTreeSet<usize>> = BTreeMap::new();
    for chunk in &plan.prune_slots {
        by_path
            .entry(plan.path.clone())
            .or_default()
            .insert(chunk.slot);
    }

    for sibling in sibling_region_files(&plan.path)? {
        for chunk in &plan.prune_slots {
            by_path
                .entry(sibling.clone())
                .or_default()
                .insert(chunk.slot);
        }
    }

    for (path, slots) in by_path {
        let mutations: Vec<(usize, SlotState)> = slots
            .into_iter()
            .map(|slot| (slot, SlotState::Empty))
            .collect();
        apply_slot_mutations(&path, &mutations)?;
    }
    Ok(())
}

fn sibling_region_files(region_path: &Path) -> Result<Vec<PathBuf>> {
    let Some(region_dir) = region_path.parent() else {
        return Ok(Vec::new());
    };
    if region_dir.file_name().and_then(|s| s.to_str()) != Some("region") {
        return Ok(Vec::new());
    }
    let Some(dim_dir) = region_dir.parent() else {
        return Ok(Vec::new());
    };
    let Some(name) = region_path.file_name() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for sibling_name in ["entities", "poi"] {
        let sibling = dim_dir.join(sibling_name).join(name);
        if sibling.is_file() {
            out.push(sibling);
        }
    }
    Ok(out)
}

fn print_plan(plan: &RegionPlan, mode: PruneMode, dry_run: bool) {
    match mode {
        PruneMode::Chunks => {
            for chunk in &plan.prune_slots {
                println!(
                    "{} chunk {},{} rel {},{} InhabitedTime={}{}",
                    plan.path.display(),
                    chunk.chunk_x,
                    chunk.chunk_z,
                    chunk.rel_x,
                    chunk.rel_z,
                    chunk.inhabited_time,
                    if dry_run { " (dry-run)" } else { "" }
                );
            }
        }
        PruneMode::Regions => {
            let max_time = plan
                .prune_slots
                .iter()
                .map(|c| c.inhabited_time)
                .max()
                .unwrap_or(0);
            println!(
                "{} region {},{} chunks={} max_InhabitedTime={}{}",
                plan.path.display(),
                plan.region_x,
                plan.region_z,
                plan.prune_slots.len(),
                max_time,
                if dry_run { " (dry-run)" } else { "" }
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::region_io::slot_index;

    #[test]
    fn reads_current_inhabited_time() {
        let bytes = nbt_with_root_long("InhabitedTime", 1234);
        assert_eq!(read_inhabited_time(&bytes).unwrap(), 1234);
    }

    #[test]
    fn reads_legacy_wrapped_inhabited_time() {
        let mut bytes = Vec::new();
        bytes.push(10);
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.push(10);
        bytes.extend_from_slice(&5u16.to_be_bytes());
        bytes.extend_from_slice(b"Level");
        bytes.push(4);
        bytes.extend_from_slice(&13u16.to_be_bytes());
        bytes.extend_from_slice(b"InhabitedTime");
        bytes.extend_from_slice(&77i64.to_be_bytes());
        bytes.push(0);
        bytes.push(0);
        assert_eq!(read_inhabited_time(&bytes).unwrap(), 77);
    }

    fn nbt_with_root_long(name: &str, value: i64) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(10);
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.push(4);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes.push(0);
        bytes
    }

    #[test]
    fn slot_math_uses_region_relative_coords() {
        assert_eq!(slot_index(31, 31), 1023);
    }
}
