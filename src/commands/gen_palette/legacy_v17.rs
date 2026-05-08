// `gen-palette` for pre-1.13 worlds (1.7.10, optionally with NotEnoughIDs).
//
// 1.7.10 has no blockstate/model JSONs — block rendering is hard-coded in
// Java. Instead we:
//
//   1. Parse `level.dat` → FML `ItemData` registry → `{id: "ns:name"}`.
//   2. For each block, locate a reasonable texture in the supplied jars:
//      - Vanilla (`minecraft:`) uses the hand-curated `(name, meta) →
//        texture` table from `shared/vanilla_1x.rs`; 1.7.10 texture layout
//        is stable and small enough that this is tractable.
//      - Modded blocks fall back to filename-based matching via the
//        `legacy_pack` module.
//   3. Average each texture into a color using the shared `avg_colour`.
//   4. Apply biome tints + water/lava special-cases via the shared
//      `vanilla_1x` post-processor.
//   5. Emit a JSON palette wrapped with `"format": "1.7.10"` so the renderer
//      can distinguish it from the modern flat-map palette.

use log::{debug, info};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::legacy_pack::{MatchKind, ResolveStats, TexturePack, load_texture_packs, resolve_modded};
use super::leveldat::{FmlRegistry17, load_fml_registry_v17};
use super::shared::color::avg_colour;
use super::shared::output::PaletteOutput;
use super::shared::overrides::{apply_overrides, load_overrides};
use super::shared::progress::PackLoadReport;
use super::shared::vanilla_1x;
use crate::anvil::legacy::palette::LegacyPaletteFile;
use crate::anvil::palette::Rgba;
use crate::output::emit_if_json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const FALLBACK_GRAY: Rgba = [128, 128, 128, 255];

#[derive(Serialize)]
struct RegistryLoadedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    blocks: usize,
    items: usize,
}

#[derive(Serialize)]
struct PackLoadedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    path: String,
    index: usize,
    total: usize,
    blockstates_added: usize,
    models_added: usize,
    textures_added: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[derive(Serialize)]
struct PacksDoneEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    pack_count: usize,
    textures: usize,
}

#[derive(Serialize, Clone, Copy)]
struct LegacyCounters {
    vanilla: usize,
    modded_exact: usize,
    modded_fuzzy: usize,
    fallback: usize,
    missing: usize,
    malformed: usize,
}

impl From<&ResolveStats> for LegacyCounters {
    fn from(s: &ResolveStats) -> Self {
        Self {
            vanilla: s.vanilla,
            modded_exact: s.modded_exact,
            modded_fuzzy: s.modded_fuzzy,
            fallback: s.fallback,
            missing: s.missing,
            malformed: s.malformed,
        }
    }
}

#[derive(Serialize)]
struct ResolvedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    counters: LegacyCounters,
}

#[derive(Serialize)]
struct OverridesAppliedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    count: usize,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    output: String,
    entries: usize,
    counters: LegacyCounters,
}

pub fn run_legacy_v17(
    packs: &[PathBuf],
    level_dat: &Path,
    output: &Path,
    overrides: Option<&Path>,
) -> Result<()> {
    info!("Starting legacy palette generation (1.7.10)");
    info!("Level.dat: {}", level_dat.display());
    info!("Packs ({}):", packs.len());
    for p in packs {
        info!("  - {}", p.display());
    }
    if let Some(o) = overrides {
        info!("Overrides: {}", o.display());
    }
    info!("Output: {}", output.display());

    let registry: FmlRegistry17 = load_fml_registry_v17(level_dat)?;
    info!(
        "FML registry: {} blocks, {} items",
        registry.blocks.len(),
        registry.items.len()
    );
    emit_if_json(&RegistryLoadedEvent {
        ty: "progress",
        phase: "registry_loaded",
        blocks: registry.blocks.len(),
        items: registry.items.len(),
    });

    let texture_packs = load_texture_packs(packs, |report: &PackLoadReport| {
        emit_if_json(&PackLoadedEvent {
            ty: "progress",
            phase: "pack_loaded",
            path: report.path.display().to_string(),
            index: report.index,
            total: report.total,
            blockstates_added: report.blockstates_added,
            models_added: report.models_added,
            textures_added: report.textures_added,
            error: report.error.as_deref(),
        });
    })?;
    let total_textures: usize = texture_packs.iter().map(|p| p.textures.len()).sum();
    info!(
        "Loaded {} texture pack(s), {} total textures",
        texture_packs.len(),
        total_textures
    );
    emit_if_json(&PacksDoneEvent {
        ty: "progress",
        phase: "packs_done",
        pack_count: texture_packs.len(),
        textures: total_textures,
    });

    let mut palette: HashMap<String, Rgba> = HashMap::new();
    let mut stats = ResolveStats::default();

    // Track which (numeric id) keys came from which `namespace:name` so we
    // can apply name-driven post-processing (biome tints, water/lava fixups)
    // at the end.
    let mut id_to_name: HashMap<u16, String> = HashMap::new();

    for (id, name) in sort_by_id(&registry.blocks) {
        if name.is_empty() {
            continue;
        }
        if let Some((ns, local)) = name.split_once(':') {
            if ns == "minecraft" {
                emit_vanilla_block(id, local, &texture_packs, &mut palette, &mut stats);
            } else {
                emit_modded_block(id, ns, local, &texture_packs, &mut palette, &mut stats);
            }
            id_to_name.insert(id, name.to_string());
        } else {
            debug!("Skipping malformed block name: {}", name);
            stats.malformed += 1;
        }
    }

    vanilla_1x::apply_vanilla_postprocess(&mut palette, &id_to_name);

    let counters = LegacyCounters::from(&stats);
    emit_if_json(&ResolvedEvent {
        ty: "progress",
        phase: "resolved",
        counters,
    });

    if let Some(path) = overrides {
        let overrides_map = load_overrides(path)?;
        let n = apply_overrides(&mut palette, overrides_map);
        info!("Applied {} override entries", n);
        emit_if_json(&OverridesAppliedEvent {
            ty: "progress",
            phase: "overrides_applied",
            count: n,
        });
    }

    info!(
        "Resolved: {} vanilla, {} modded (exact), {} modded (fuzzy), {} fallback, {} missing, {} malformed",
        stats.vanilla,
        stats.modded_exact,
        stats.modded_fuzzy,
        stats.fallback,
        stats.missing,
        stats.malformed
    );

    let file = LegacyPaletteFile {
        format: "1.7.10".to_string(),
        blocks: palette,
    };
    let out = PaletteOutput::Legacy(&file);
    out.write_to(output)?;
    info!(
        "Legacy palette generation complete — {} entries written to {}",
        out.entry_count(),
        output.display()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        output: output.display().to_string(),
        entries: out.entry_count(),
        counters,
    });

    Ok(())
}

fn sort_by_id(blocks: &HashMap<u16, String>) -> Vec<(u16, &str)> {
    let mut v: Vec<(u16, &str)> = blocks.iter().map(|(k, v)| (*k, v.as_str())).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}

/// Vanilla path: look up (name, meta) → texture in the hand-curated table.
/// Emits one entry per registered meta, plus a bare-id fallback.
fn emit_vanilla_block(
    id: u16,
    local: &str,
    packs: &[TexturePack],
    palette: &mut HashMap<String, Rgba>,
    stats: &mut ResolveStats,
) {
    let variants = vanilla_1x::variants_for(local);
    if variants.is_empty() {
        // No table entry. Try the vanilla pack's filename directly.
        let key = format!("minecraft:block/{}", local);
        if let Some(color) = lookup_texture(packs, &key) {
            palette.insert(format!("{}", id), color);
            stats.vanilla += 1;
        } else {
            debug!("No vanilla texture for minecraft:{}", local);
            palette.insert(format!("{}", id), FALLBACK_GRAY);
            stats.missing += 1;
        }
        return;
    }

    let mut any_resolved = false;
    let mut first_color = None;
    for (meta, texture_path) in variants {
        if let Some(color) = lookup_texture(packs, texture_path) {
            palette.insert(format!("{}|{}", id, meta), color);
            any_resolved = true;
            if first_color.is_none() {
                first_color = Some(color);
            }
        } else {
            debug!("Vanilla texture not found: {}", texture_path);
        }
    }
    if any_resolved {
        palette.insert(format!("{}", id), first_color.unwrap());
        stats.vanilla += 1;
    } else {
        palette.insert(format!("{}", id), FALLBACK_GRAY);
        stats.missing += 1;
    }
}

/// Modded path: filename-based fuzzy match inside the namespaced jar.
fn emit_modded_block(
    id: u16,
    ns: &str,
    local: &str,
    packs: &[TexturePack],
    palette: &mut HashMap<String, Rgba>,
    stats: &mut ResolveStats,
) {
    match resolve_modded(ns, local, packs) {
        Some((color, how)) => {
            palette.insert(format!("{}", id), color);
            match how {
                MatchKind::Exact => stats.modded_exact += 1,
                MatchKind::Fuzzy => stats.modded_fuzzy += 1,
            }
        }
        None => {
            palette.insert(format!("{}", id), FALLBACK_GRAY);
            stats.fallback += 1;
            debug!("No texture found for {}:{}", ns, local);
        }
    }
}

fn lookup_texture(packs: &[TexturePack], key: &str) -> Option<Rgba> {
    for p in packs {
        if let Some(tex) = p.textures.get(key) {
            return Some(avg_colour(tex));
        }
    }
    None
}
