// `gen-palette legacy` — palette generation for pre-1.13 worlds (1.7.10,
// optionally with NotEnoughIDs).
//
// The modern gen-palette walks blockstate/model/texture JSONs. 1.7.10 has
// none of those — block rendering is all hard-coded in Java. Instead we:
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

mod leveldat;

use clap::Args;
use log::{debug, info};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use super::legacy_pack::{MatchKind, ResolveStats, TexturePack, load_texture_packs, resolve_modded};
use super::shared::color::avg_colour;
use super::shared::overrides::{apply_overrides, load_overrides};
use super::shared::progress::PackLoadReport;
use super::shared::vanilla_1x;
use crate::anvil::legacy::palette::LegacyPaletteFile;
use crate::anvil::palette::Rgba;
use crate::chown;
use crate::output::emit_if_json;
use leveldat::{FmlRegistry, load_fml_registry};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct LegacyArgs {
    /// Path to the world's `level.dat`. The FML block registry inside it
    /// defines the (id → name) mapping the palette is keyed by.
    #[arg(short, long)]
    level_dat: PathBuf,

    /// Resource pack: a .jar/.zip file or a directory containing them.
    /// Repeatable — first-listed wins on filename conflicts (list custom
    /// resource packs first, vanilla last).
    #[arg(short, long, required = true)]
    pack: Vec<PathBuf>,

    /// Output palette.json file path.
    #[arg(short, long, default_value = "palette.json")]
    output: PathBuf,

    /// Optional user overrides — JSON map of `"id"` or `"id|meta"` →
    /// `[r,g,b,a]`. Applied last, takes precedence over everything automatic.
    #[arg(long)]
    overrides: Option<PathBuf>,
}

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

pub fn execute(args: LegacyArgs) -> Result<()> {
    info!("Starting legacy palette generation (1.7.10)");
    info!("Level.dat: {}", args.level_dat.display());
    info!("Packs ({}):", args.pack.len());
    for p in &args.pack {
        info!("  - {}", p.display());
    }
    if let Some(ref o) = args.overrides {
        info!("Overrides: {}", o.display());
    }
    info!("Output: {}", args.output.display());

    let registry: FmlRegistry = load_fml_registry(&args.level_dat)?;
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

    let packs = load_texture_packs(&args.pack, |report: &PackLoadReport| {
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
    let total_textures: usize = packs.iter().map(|p| p.textures.len()).sum();
    info!(
        "Loaded {} texture pack(s), {} total textures",
        packs.len(),
        total_textures
    );
    emit_if_json(&PacksDoneEvent {
        ty: "progress",
        phase: "packs_done",
        pack_count: packs.len(),
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
                emit_vanilla_block(id, local, &packs, &mut palette, &mut stats);
            } else {
                emit_modded_block(id, ns, local, &packs, &mut palette, &mut stats);
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

    if let Some(ref path) = args.overrides {
        let overrides = load_overrides(path)?;
        let n = apply_overrides(&mut palette, overrides);
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
    let bytes = serde_json::to_vec_pretty(&file)?;
    std::fs::write(&args.output, &bytes)?;
    chown::apply(&args.output)
        .map_err(|e| format!("Failed to chown {}: {}", args.output.display(), e))?;
    info!(
        "Legacy palette generation complete — {} entries written to {}",
        file.blocks.len(),
        args.output.display()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        output: args.output.display().to_string(),
        entries: file.blocks.len(),
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
