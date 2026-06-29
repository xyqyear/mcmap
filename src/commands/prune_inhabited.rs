use clap::{Args, ValueEnum};
use flate2::read::{GzDecoder, ZlibDecoder};
use log::{info, warn};
use lz4_java_wrc::Lz4BlockInput;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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

    /// Protect chunks claimed by an extract-ftb-claims JSON file or NDJSON stream.
    /// Use `-` to read from stdin.
    #[arg(long, value_name = "FILE|-")]
    exclude_ftb_claims: Option<PathBuf>,
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
struct RegionFileSet {
    path: PathBuf,
    files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct RegionPlan {
    path: PathBuf,
    region_x: isize,
    region_z: isize,
    present_chunks: usize,
    prune_slots: Vec<ChunkPlan>,
    chunks_skipped_by_claims: usize,
    region_skipped_by_claims: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ChunkCoord {
    x: i64,
    z: i64,
}

#[derive(Debug)]
struct ClaimProtection {
    claims_loaded: usize,
    claimed_chunks_protected: usize,
    by_region_dir: BTreeMap<PathBuf, BTreeSet<ChunkCoord>>,
}

#[derive(Deserialize, Debug)]
struct FtbClaimsPayload {
    world_dir: String,
    dimensions: Vec<FtbDimension>,
    teams: Vec<FtbTeam>,
}

#[derive(Deserialize, Debug)]
struct FtbDimension {
    id: String,
    folder: String,
}

#[derive(Deserialize, Debug)]
struct FtbTeam {
    #[serde(default)]
    claims: Vec<FtbClaim>,
}

#[derive(Deserialize, Debug)]
struct FtbClaim {
    dim: String,
    cx: i32,
    cz: i32,
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
struct ProgressEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    regions_processed: usize,
    regions_total: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    claims_loaded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    claimed_chunks_protected: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunks_skipped_by_claims: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    regions_skipped_by_claims: Option<usize>,
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

    let claim_protection = match &args.exclude_ftb_claims {
        Some(path) => Some(load_claim_protection(path, &args.path, &region_dirs)?),
        None => None,
    };
    if let Some(protection) = &claim_protection {
        info!(
            "Loaded {} FTB claims; protecting {} claimed chunks",
            protection.claims_loaded, protection.claimed_chunks_protected
        );
    }

    let region_file_sets = collect_region_file_sets(&region_dirs)?;
    let regions_total: usize = region_file_sets.iter().map(|set| set.files.len()).sum();
    for set in &region_file_sets {
        if is_json() {
            emit(&RegionDirEvent {
                ty: "region_dir",
                path: set.path.display().to_string(),
                regions: set.files.len(),
            });
        }
    }

    let progress_phase = if args.dry_run { "scan" } else { "prune" };
    let mut regions_scanned = 0usize;
    let mut regions_processed = 0usize;
    let mut chunks_scanned = 0usize;
    let mut chunks_selected = 0usize;
    let mut selected_region_count = 0usize;
    let mut chunks_skipped_by_claims = 0usize;
    let mut regions_skipped_by_claims = 0usize;

    for set in &region_file_sets {
        let region_dir_key = canonicalize_existing(&set.path);
        let protected_chunks = claim_protection
            .as_ref()
            .and_then(|p| p.by_region_dir.get(&region_dir_key));

        for region_path in &set.files {
            match plan_region(&region_path, args.threshold, args.mode, protected_chunks) {
                Ok(Some(plan)) => {
                    regions_scanned += 1;
                    chunks_scanned += plan.present_chunks;
                    chunks_selected += plan.prune_slots.len();
                    chunks_skipped_by_claims += plan.chunks_skipped_by_claims;
                    if plan.region_skipped_by_claims {
                        regions_skipped_by_claims += 1;
                    }
                    if !plan.prune_slots.is_empty() {
                        selected_region_count += 1;
                        emit_or_print_plan(&plan, args.mode, args.dry_run);
                        if !args.dry_run {
                            apply_plan(&plan)?;
                        }
                    }
                }
                Ok(None) => regions_scanned += 1,
                Err(e) => warn!("{}: {}", region_path.display(), e),
            }
            regions_processed += 1;
            emit_progress(progress_phase, regions_processed, regions_total);
        }
    }

    if claim_protection.is_some() && (chunks_skipped_by_claims > 0 || regions_skipped_by_claims > 0)
    {
        info!(
            "Skipped {} chunks and {} regions because of FTB claims",
            chunks_skipped_by_claims, regions_skipped_by_claims
        );
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
            claims_loaded: claim_protection.as_ref().map(|p| p.claims_loaded),
            claimed_chunks_protected: claim_protection
                .as_ref()
                .map(|p| p.claimed_chunks_protected),
            chunks_skipped_by_claims: claim_protection.as_ref().map(|_| chunks_skipped_by_claims),
            regions_skipped_by_claims: claim_protection.as_ref().map(|_| regions_skipped_by_claims),
        });
    }

    Ok(())
}

fn collect_region_file_sets(region_dirs: &[RegionDir]) -> Result<Vec<RegionFileSet>> {
    region_dirs
        .iter()
        .map(|dir| {
            Ok(RegionFileSet {
                path: dir.path.clone(),
                files: list_region_files(&dir.path)?,
            })
        })
        .collect()
}

fn emit_or_print_plan(plan: &RegionPlan, mode: PruneMode, dry_run: bool) {
    if is_json() {
        match mode {
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
                        dry_run,
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
                    dry_run,
                });
            }
        }
    } else {
        print_plan(plan, mode, dry_run);
    }
}

fn emit_progress(phase: &'static str, regions_processed: usize, regions_total: usize) {
    if is_json() {
        emit(&ProgressEvent {
            ty: "progress",
            phase,
            regions_processed,
            regions_total,
        });
    }
}

fn load_claim_protection(
    input: &Path,
    prune_root: &Path,
    region_dirs: &[RegionDir],
) -> Result<ClaimProtection> {
    let text = read_claim_input(input)?;
    let payload = parse_ftb_claims_payload(&text)?;
    let claims_loaded: usize = payload.teams.iter().map(|t| t.claims.len()).sum();

    let mut claims_by_dim: HashMap<String, BTreeSet<ChunkCoord>> = HashMap::new();
    for team in &payload.teams {
        for claim in &team.claims {
            claims_by_dim
                .entry(claim.dim.clone())
                .or_default()
                .insert(ChunkCoord {
                    x: claim.cx as i64,
                    z: claim.cz as i64,
                });
        }
    }

    let claim_world_dir = PathBuf::from(payload.world_dir);
    let dim_by_id: HashMap<String, FtbDimension> = payload
        .dimensions
        .into_iter()
        .map(|dim| (dim.id.clone(), dim))
        .collect();
    let mut by_region_dir: BTreeMap<PathBuf, BTreeSet<ChunkCoord>> = BTreeMap::new();

    for (dim_id, claims) in claims_by_dim {
        let Some(dim) = dim_by_id.get(&dim_id) else {
            warn!(
                "FTB claim dimension {} has no dimensions[] entry; skipping",
                dim_id
            );
            continue;
        };
        let matching_region_dirs =
            matching_region_dirs(region_dirs, &claim_world_dir, prune_root, &dim.folder);
        for region_dir in matching_region_dirs {
            by_region_dir
                .entry(region_dir)
                .or_default()
                .extend(claims.iter().copied());
        }
    }

    let claimed_chunks_protected = by_region_dir.values().map(BTreeSet::len).sum();
    Ok(ClaimProtection {
        claims_loaded,
        claimed_chunks_protected,
        by_region_dir,
    })
}

fn read_claim_input(input: &Path) -> Result<String> {
    let mut text = String::new();
    if input == Path::new("-") {
        std::io::stdin().read_to_string(&mut text)?;
        return Ok(text);
    }
    std::fs::File::open(input)?.read_to_string(&mut text)?;
    Ok(text)
}

fn parse_ftb_claims_payload(text: &str) -> Result<FtbClaimsPayload> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        return parse_ftb_claims_value(value);
    }

    let mut saw_json_line = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        saw_json_line = true;
        if value.get("type").and_then(|v| v.as_str()) != Some("result") {
            continue;
        }
        if let Some(data) = value.get("data") {
            return Ok(serde_json::from_value(data.clone())?);
        }
    }

    if saw_json_line {
        Err("FTB claims NDJSON stream did not contain a result.data payload".into())
    } else {
        Err("FTB claims input is not valid JSON".into())
    }
}

fn parse_ftb_claims_value(value: serde_json::Value) -> Result<FtbClaimsPayload> {
    if value.get("type").and_then(|v| v.as_str()) == Some("result") {
        let data = value
            .get("data")
            .ok_or("extract-ftb-claims result event has no data field")?;
        return Ok(serde_json::from_value(data.clone())?);
    }
    Ok(serde_json::from_value(value)?)
}

fn matching_region_dirs(
    region_dirs: &[RegionDir],
    claim_world_dir: &Path,
    prune_root: &Path,
    dim_folder: &str,
) -> Vec<PathBuf> {
    let exact: Vec<PathBuf> = region_dirs
        .iter()
        .filter_map(|dir| {
            region_dir_matches_exact(&dir.path, claim_world_dir, prune_root, dim_folder)
                .then(|| canonicalize_existing(&dir.path))
        })
        .collect();
    if !exact.is_empty() || dim_folder == "." {
        return exact;
    }

    region_dirs
        .iter()
        .filter_map(|dir| {
            region_dir_has_folder_suffix(&dir.path, dim_folder)
                .then(|| canonicalize_existing(&dir.path))
        })
        .collect()
}

fn region_dir_matches_exact(
    region_dir: &Path,
    claim_world_dir: &Path,
    prune_root: &Path,
    dim_folder: &str,
) -> bool {
    let actual = canonicalize_existing(region_dir);
    let mut candidates = vec![
        region_dir_for_dimension(claim_world_dir, dim_folder),
        region_dir_for_dimension(prune_root, dim_folder),
    ];
    if dim_folder == "." && prune_root.file_name().and_then(|s| s.to_str()) == Some("region") {
        candidates.push(prune_root.to_path_buf());
    }

    candidates
        .into_iter()
        .map(|p| canonicalize_existing(&p))
        .any(|candidate| paths_equal(&actual, &candidate))
}

fn region_dir_for_dimension(base: &Path, dim_folder: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    if dim_folder != "." {
        for part in dim_folder.split('/').filter(|part| !part.is_empty()) {
            if part != "." {
                path.push(part);
            }
        }
    }
    path.join("region")
}

fn region_dir_has_folder_suffix(region_dir: &Path, dim_folder: &str) -> bool {
    let mut suffix: Vec<String> = dim_folder
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| part.to_string())
        .collect();
    suffix.push("region".to_string());

    let parts: Vec<String> = region_dir
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    path_components_end_with(&parts, &suffix)
}

fn path_components_end_with(parts: &[String], suffix: &[String]) -> bool {
    if suffix.len() > parts.len() {
        return false;
    }
    parts[parts.len() - suffix.len()..]
        .iter()
        .zip(suffix)
        .all(|(a, b)| path_component_eq(a, b))
}

fn canonicalize_existing(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(windows)]
fn paths_equal(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(not(windows))]
fn paths_equal(a: &Path, b: &Path) -> bool {
    a == b
}

#[cfg(windows)]
fn path_component_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(not(windows))]
fn path_component_eq(a: &str, b: &str) -> bool {
    a == b
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

fn plan_region(
    region_path: &Path,
    threshold: i64,
    mode: PruneMode,
    protected_chunks: Option<&BTreeSet<ChunkCoord>>,
) -> Result<Option<RegionPlan>> {
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

    let (prune_slots, chunks_skipped_by_claims, region_skipped_by_claims) =
        select_prune_slots(&present, threshold, mode, protected_chunks, rx, rz);

    Ok(Some(RegionPlan {
        path: region_path.to_path_buf(),
        region_x: rx,
        region_z: rz,
        present_chunks: present.len(),
        prune_slots,
        chunks_skipped_by_claims,
        region_skipped_by_claims,
    }))
}

fn select_prune_slots(
    present: &[ChunkPlan],
    threshold: i64,
    mode: PruneMode,
    protected_chunks: Option<&BTreeSet<ChunkCoord>>,
    region_x: isize,
    region_z: isize,
) -> (Vec<ChunkPlan>, usize, bool) {
    match mode {
        PruneMode::Chunks => {
            let mut skipped = 0usize;
            let prune_slots = present
                .iter()
                .filter(|chunk| chunk.inhabited_time < threshold)
                .filter_map(|chunk| {
                    if is_claimed_chunk(chunk, protected_chunks) {
                        skipped += 1;
                        None
                    } else {
                        Some(chunk.clone())
                    }
                })
                .collect();
            (prune_slots, skipped, false)
        }
        PruneMode::Regions => {
            if !present.iter().all(|chunk| chunk.inhabited_time < threshold) {
                return (Vec::new(), 0, false);
            }
            if region_contains_claim(region_x, region_z, protected_chunks) {
                return (Vec::new(), present.len(), true);
            }
            (present.to_vec(), 0, false)
        }
    }
}

fn is_claimed_chunk(chunk: &ChunkPlan, protected_chunks: Option<&BTreeSet<ChunkCoord>>) -> bool {
    protected_chunks.is_some_and(|claims| {
        claims.contains(&ChunkCoord {
            x: chunk.chunk_x as i64,
            z: chunk.chunk_z as i64,
        })
    })
}

fn region_contains_claim(
    region_x: isize,
    region_z: isize,
    protected_chunks: Option<&BTreeSet<ChunkCoord>>,
) -> bool {
    let Some(claims) = protected_chunks else {
        return false;
    };
    let min_x = region_x as i64 * 32;
    let min_z = region_z as i64 * 32;
    let max_x = min_x + 31;
    let max_z = min_z + 31;
    claims
        .iter()
        .any(|claim| claim.x >= min_x && claim.x <= max_x && claim.z >= min_z && claim.z <= max_z)
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
    use crate::commands::region_io::{emit_region, slot_index};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

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

    #[test]
    fn parses_ftb_claims_result_event() {
        let text = r#"{"type":"result","data":{"world_dir":"/world","dimensions":[{"id":"minecraft:overworld","folder":".","exists":true}],"teams":[{"claims":[{"dim":"minecraft:overworld","cx":4,"cz":-7,"force_loaded":false}]}]}}"#;
        let payload = parse_ftb_claims_payload(text).unwrap();
        assert_eq!(payload.world_dir, "/world");
        assert_eq!(payload.dimensions[0].folder, ".");
        assert_eq!(payload.teams[0].claims[0].cx, 4);
        assert_eq!(payload.teams[0].claims[0].cz, -7);
    }

    #[test]
    fn parses_ftb_claims_plain_payload() {
        let text = r#"{"world_dir":"/world","dimensions":[{"id":"minecraft:overworld","folder":".","exists":true}],"teams":[{"claims":[{"dim":"minecraft:overworld","cx":4,"cz":15,"force_loaded":false}]}]}"#;
        let payload = parse_ftb_claims_payload(text).unwrap();
        assert_eq!(payload.world_dir, "/world");
        assert_eq!(payload.teams[0].claims.len(), 1);
    }

    #[test]
    fn parses_ftb_claims_ndjson_stream() {
        let text = r#"{"type":"progress","phase":"ignored"}
{"type":"result","data":{"world_dir":"/world","dimensions":[{"id":"minecraft:overworld","folder":".","exists":true}],"teams":[{"claims":[{"dim":"minecraft:overworld","cx":4,"cz":15,"force_loaded":false}]}]}}"#;
        let payload = parse_ftb_claims_payload(text).unwrap();
        assert_eq!(payload.world_dir, "/world");
        assert_eq!(payload.teams[0].claims[0].cx, 4);
    }

    #[test]
    fn rejects_ftb_claims_ndjson_without_result_data() {
        let text = r#"{"type":"progress","phase":"ignored"}
{"type":"progress","phase":"still_ignored"}"#;
        let err = parse_ftb_claims_payload(text).unwrap_err();
        assert!(
            err.to_string()
                .contains("did not contain a result.data payload")
        );
    }

    #[test]
    fn load_claim_protection_maps_dimensions_and_deduplicates() {
        let temp = TempDir::new("claim-map");
        let world = temp.path().join("world");
        let dirs = [
            world.join("region"),
            world.join("DIM-1").join("region"),
            world
                .join("dimensions")
                .join("allthemodium")
                .join("mining")
                .join("region"),
        ];
        for dir in &dirs {
            fs::create_dir_all(dir).unwrap();
        }
        let region_dirs: Vec<RegionDir> = dirs
            .iter()
            .map(|path| RegionDir { path: path.clone() })
            .collect();
        let claims_path = temp.path().join("claims.json");
        write_claims_file(
            &claims_path,
            &world,
            serde_json::json!([
                {"id":"minecraft:overworld","folder":".","exists":true},
                {"id":"minecraft:the_nether","folder":"DIM-1","exists":true},
                {"id":"allthemodium:mining","folder":"dimensions/allthemodium/mining","exists":true}
            ]),
            serde_json::json!([
                {"dim":"minecraft:overworld","cx":4,"cz":15,"force_loaded":false},
                {"dim":"minecraft:overworld","cx":4,"cz":15,"force_loaded":true},
                {"dim":"minecraft:the_nether","cx":7,"cz":1,"force_loaded":false},
                {"dim":"allthemodium:mining","cx":9,"cz":1,"force_loaded":false}
            ]),
        );

        let protection = load_claim_protection(&claims_path, &world, &region_dirs).unwrap();

        assert_eq!(protection.claims_loaded, 4);
        assert_eq!(protection.claimed_chunks_protected, 3);
        assert!(
            protection.by_region_dir[&canonicalize_existing(&dirs[0])]
                .contains(&ChunkCoord { x: 4, z: 15 })
        );
        assert!(
            protection.by_region_dir[&canonicalize_existing(&dirs[1])]
                .contains(&ChunkCoord { x: 7, z: 1 })
        );
        assert!(
            protection.by_region_dir[&canonicalize_existing(&dirs[2])]
                .contains(&ChunkCoord { x: 9, z: 1 })
        );
    }

    #[test]
    fn load_claim_protection_matches_dimension_suffix_for_dimension_roots() {
        let temp = TempDir::new("claim-suffix");
        let claim_world = temp.path().join("old").join("world");
        let prune_root = temp
            .path()
            .join("copy")
            .join("world")
            .join("dimensions")
            .join("allthemodium")
            .join("mining");
        let region_dir = prune_root.join("region");
        fs::create_dir_all(&region_dir).unwrap();
        let region_dirs = vec![RegionDir {
            path: region_dir.clone(),
        }];
        let claims_path = temp.path().join("claims.json");
        write_claims_file(
            &claims_path,
            &claim_world,
            serde_json::json!([
                {"id":"allthemodium:mining","folder":"dimensions/allthemodium/mining","exists":true}
            ]),
            serde_json::json!([
                {"dim":"allthemodium:mining","cx":9,"cz":1,"force_loaded":false}
            ]),
        );

        let protection = load_claim_protection(&claims_path, &prune_root, &region_dirs).unwrap();

        assert_eq!(protection.claimed_chunks_protected, 1);
        assert!(
            protection.by_region_dir[&canonicalize_existing(&region_dir)]
                .contains(&ChunkCoord { x: 9, z: 1 })
        );
    }

    #[test]
    fn chunks_mode_excludes_claimed_chunks() {
        let present = vec![
            chunk_plan(4, 15, 480),
            chunk_plan(5, 15, 480),
            chunk_plan(6, 15, 2400),
        ];
        let protected = BTreeSet::from([ChunkCoord { x: 4, z: 15 }]);

        let (prune, skipped, skipped_region) =
            select_prune_slots(&present, 1200, PruneMode::Chunks, Some(&protected), 0, 0);

        assert_eq!(prune.len(), 1);
        assert_eq!(prune[0].chunk_x, 5);
        assert_eq!(skipped, 1);
        assert!(!skipped_region);
    }

    #[test]
    fn regions_mode_skips_region_containing_claim() {
        let present = vec![chunk_plan(0, 0, 480), chunk_plan(1, 0, 480)];
        let protected = BTreeSet::from([ChunkCoord { x: 12, z: 12 }]);

        let (prune, skipped, skipped_region) =
            select_prune_slots(&present, 1200, PruneMode::Regions, Some(&protected), 0, 0);

        assert!(prune.is_empty());
        assert_eq!(skipped, present.len());
        assert!(skipped_region);
    }

    #[test]
    fn execute_excludes_claimed_chunks_and_prunes_siblings() {
        let temp = TempDir::new("claim-execute-chunks");
        let world = temp.path().join("world");
        let chunks = [(4, 15, 480), (5, 15, 480), (6, 15, 2400)];
        for subdir in ["region", "entities", "poi"] {
            write_region_file(&world.join(subdir).join("r.0.0.mca"), &chunks);
        }
        let claims_path = temp.path().join("claims.json");
        write_claims_file(
            &claims_path,
            &world,
            serde_json::json!([
                {"id":"minecraft:overworld","folder":".","exists":true}
            ]),
            serde_json::json!([
                {"dim":"minecraft:overworld","cx":4,"cz":15,"force_loaded":false}
            ]),
        );

        execute(PruneInhabitedArgs {
            path: world.clone(),
            threshold: 1200,
            mode: PruneMode::Chunks,
            dry_run: false,
            exclude_ftb_claims: Some(claims_path),
        })
        .unwrap();

        for subdir in ["region", "entities", "poi"] {
            let path = world.join(subdir).join("r.0.0.mca");
            assert!(slot_present(&path, 4, 15));
            assert!(!slot_present(&path, 5, 15));
            assert!(slot_present(&path, 6, 15));
        }
    }

    #[test]
    fn execute_region_mode_skips_claimed_region() {
        let temp = TempDir::new("claim-execute-regions");
        let world = temp.path().join("world");
        let unclaimed_region = world.join("region").join("r.1.0.mca");
        let claimed_region = world.join("region").join("r.2.0.mca");
        write_region_file(&unclaimed_region, &[(8, 0, 480), (9, 0, 480)]);
        write_region_file(&claimed_region, &[(0, 0, 480), (1, 0, 480)]);
        let claims_path = temp.path().join("claims.json");
        write_claims_file(
            &claims_path,
            &world,
            serde_json::json!([
                {"id":"minecraft:overworld","folder":".","exists":true}
            ]),
            serde_json::json!([
                {"dim":"minecraft:overworld","cx":64,"cz":0,"force_loaded":false}
            ]),
        );

        execute(PruneInhabitedArgs {
            path: world,
            threshold: 1200,
            mode: PruneMode::Regions,
            dry_run: false,
            exclude_ftb_claims: Some(claims_path),
        })
        .unwrap();

        assert!(!slot_present(&unclaimed_region, 8, 0));
        assert!(!slot_present(&unclaimed_region, 9, 0));
        assert!(slot_present(&claimed_region, 0, 0));
        assert!(slot_present(&claimed_region, 1, 0));
    }

    fn chunk_plan(chunk_x: isize, chunk_z: isize, inhabited_time: i64) -> ChunkPlan {
        ChunkPlan {
            slot: slot_index((chunk_x & 31) as u8, (chunk_z & 31) as u8),
            rel_x: (chunk_x & 31) as u8,
            rel_z: (chunk_z & 31) as u8,
            chunk_x,
            chunk_z,
            inhabited_time,
        }
    }

    fn write_claims_file(
        path: &Path,
        world: &Path,
        dimensions: serde_json::Value,
        claims: serde_json::Value,
    ) {
        let payload = serde_json::json!({
            "mcmap_extract_ftb_claims_version": 1,
            "detected_format": "snbt",
            "world_dir": world.to_string_lossy(),
            "dimensions": dimensions,
            "teams": [{
                "id": "manual-team",
                "type": "player",
                "members": [],
                "claims": claims
            }]
        });
        fs::write(path, serde_json::to_vec(&payload).unwrap()).unwrap();
    }

    fn write_region_file(path: &Path, chunks: &[(u8, u8, i64)]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut slots = vec![SlotState::Empty; SLOT_COUNT];
        for &(rel_x, rel_z, inhabited_time) in chunks {
            slots[slot_index(rel_x, rel_z)] = SlotState::Inline {
                scheme: 3,
                payload: nbt_with_root_long("InhabitedTime", inhabited_time),
                timestamp: 1,
            };
        }
        fs::write(path, emit_region(&slots).unwrap()).unwrap();
    }

    fn slot_present(path: &Path, rel_x: u8, rel_z: u8) -> bool {
        let bytes = fs::read(path).unwrap();
        !matches!(
            read_slot(
                &bytes,
                slot_index(rel_x, rel_z),
                path.parent().unwrap(),
                region_coords(path),
                false,
                "test",
            )
            .unwrap(),
            SlotState::Empty
        )
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("mcmap-prune-{label}-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
