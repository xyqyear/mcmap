// `gen-palette` for 1.13+ worlds.
//
// Walks standard blockstate/model/texture JSONs across every supplied resource
// pack (vanilla and modded are treated identically; namespace is derived from
// the on-disk path). Resolution tiers are listed at the top of
// `modern_pack/mod.rs`.

use fastanvil::{
    Rgba,
    tex::{Blockstate, Renderer, Texture},
};
use log::{info, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::modern_pack::raw::RawBlockstate;
use super::modern_pack::{
    Counters, Pools, Resolver, add_base_colors, add_missing_blocks, default_regex_mappings,
    load_packs,
};
use super::shared::output::PaletteOutput;
use super::shared::overrides::{apply_overrides, load_overrides};
use super::shared::progress::PackLoadReport;
use crate::output::emit_if_json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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
    blockstates: usize,
    models: usize,
    textures: usize,
}

#[derive(Serialize)]
struct ResolvedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    counters: ResolverCounters,
    failed: usize,
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
    failed: usize,
    counters: ResolverCounters,
}

#[derive(Serialize, Clone, Copy)]
struct ResolverCounters {
    rendered: usize,
    side_fallback: usize,
    particle: usize,
    any_texture: usize,
    regex_mapped: usize,
    probed: usize,
    substring: usize,
    generic_blockstate: usize,
}

impl From<&Counters> for ResolverCounters {
    fn from(c: &Counters) -> Self {
        Self {
            rendered: c.rendered,
            side_fallback: c.side_fallback,
            particle: c.particle,
            any_texture: c.any_texture,
            regex_mapped: c.mapped,
            probed: c.probed,
            substring: c.substring,
            generic_blockstate: c.generic_blockstate,
        }
    }
}

pub fn run_modern(packs: &[PathBuf], output: &Path, overrides: Option<&Path>) -> Result<()> {
    info!("Starting palette generation (modern / 1.13+)");
    info!("Packs ({}):", packs.len());
    for p in packs {
        info!("  - {}", p.display());
    }
    if let Some(o) = overrides {
        info!("Overrides: {}", o.display());
    }
    info!("Output: {}", output.display());

    let pools = load_packs(packs, |report: &PackLoadReport| {
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
    emit_if_json(&PacksDoneEvent {
        ty: "progress",
        phase: "packs_done",
        pack_count: packs.len(),
        blockstates: pools.blockstates.len(),
        models: pools.models.len(),
        textures: pools.textures.len(),
    });
    let Pools {
        blockstates,
        models,
        textures,
        raw_blockstates,
        raw_models,
    } = pools;

    info!("Creating renderer");
    let mut renderer = Renderer::new(blockstates.clone(), models, textures.clone());
    let mut counters = Counters::default();
    let mut failed = 0usize;

    let mappings = default_regex_mappings();
    let mut palette: HashMap<String, Rgba> = HashMap::new();

    info!("Rendering blockstates");
    let tasks = collect_resolve_tasks(&blockstates, &raw_blockstates, &textures);
    {
        let mut resolver = Resolver {
            renderer: &mut renderer,
            raw_blockstates: &raw_blockstates,
            raw_models: &raw_models,
            textures: &textures,
            mappings: &mappings,
            counters: &mut counters,
        };
        for task in &tasks {
            if let Some(col) = resolver.resolve(&task.name, task.props.as_deref()) {
                palette.insert(task.description.clone(), col);
            } else if task.warn_on_failure {
                warn!("Could not resolve: {}", task.description);
                failed += 1;
            }
            // Synthetic-name tasks silently skip on failure — they may not
            // be real registered blocks (vanilla `block/bell_floor` and
            // friends are sub-models of other blockstates).
        }
    }

    info!(
        "Resolved: {} rendered, {} side/multipart, {} particle, {} any-texture, {} regex, {} probed, {} substring, {} generic-bs, {} failed",
        counters.rendered,
        counters.side_fallback,
        counters.particle,
        counters.any_texture,
        counters.mapped,
        counters.probed,
        counters.substring,
        counters.generic_blockstate,
        failed
    );
    let resolver_counters = ResolverCounters::from(&counters);
    emit_if_json(&ResolvedEvent {
        ty: "progress",
        phase: "resolved",
        counters: resolver_counters,
        failed,
    });

    // 1.17 renamed grass_path to dirt_path. Keep the old name working.
    if let Some(path) = palette.get("minecraft:dirt_path").cloned() {
        palette.insert("minecraft:grass_path".into(), path);
    }

    add_missing_blocks(&mut palette);
    add_base_colors(&mut palette);

    if let Some(path) = overrides {
        info!("Applying overrides from {}", path.display());
        let overrides_map = load_overrides(path)?;
        let n = apply_overrides(&mut palette, overrides_map);
        info!("  Applied {} override entries", n);
        emit_if_json(&OverridesAppliedEvent {
            ty: "progress",
            phase: "overrides_applied",
            count: n,
        });
    }

    info!("Writing palette to: {}", output.display());
    let out = PaletteOutput::Modern(&palette);
    out.write_to(output)?;

    info!(
        "Palette generation complete — {} total blocks written",
        out.entry_count()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        output: output.display().to_string(),
        entries: out.entry_count(),
        failed,
        counters: resolver_counters,
    });
    Ok(())
}

/// One block-key the resolver should attempt. Every variant of every block
/// from every source becomes one of these — a flat work queue feeds a single
/// resolution loop.
struct ResolveTask {
    /// Block name passed to `Resolver::resolve` — `"<ns>:<id>"`.
    name: String,
    /// Variant property string for `Resolver::resolve` (e.g. `"facing=north"`).
    /// `None` means "no specific variant" — used for multipart blockstates and
    /// for synthetic names that have no blockstate JSON.
    props: Option<String>,
    /// Palette key — `"<name>"` or `"<name>|<props>"`. Distinct from `name`
    /// because the same `name` can produce multiple palette entries (one per
    /// variant).
    description: String,
    /// Whether a resolution miss should log + count as `failed`. Strict and
    /// raw-only blockstates do; synthetic names from texture paths do not
    /// (they may not correspond to real registered blocks at all).
    warn_on_failure: bool,
}

/// Build the work queue from all three sources:
///
///  1. Strict blockstates — `fastanvil::Blockstate` parses them. `Variants`
///     produces one task per property combo; `Multipart` produces one task
///     with `props=None` (fastanvil can't render multipart, so the resolver
///     drops straight to the raw-model fallback).
///  2. Raw-only blockstates — names that only round-tripped through our
///     lenient parser (e.g. negative `x`/`y` rotations that fastanvil's
///     `Variant` rejects: `atum:deadwood_branch`,
///     `bumblezone:porous_honeycomb_block`). Same task shape, but the
///     `fastanvil::Renderer` path is unreachable; the resolver still works
///     because tier-1+ falls back to raw models.
///  3. Synthetic names from texture paths. Many mods register blocks
///     dynamically in Java without shipping a blockstate JSON (TFC's
///     path-namespaced rocks `tfc:rock/raw/andesite`, TheAbyss fluids
///     `theabyss:areno`, etc.). The texture is always
///     `assets/<ns>/textures/(block|blocks)/<rest>.png`, so synthesize
///     `<ns>:<rest>` for any such key not already covered above.
fn collect_resolve_tasks(
    blockstates: &HashMap<String, Blockstate>,
    raw_blockstates: &HashMap<String, RawBlockstate>,
    textures: &HashMap<String, Texture>,
) -> Vec<ResolveTask> {
    let mut tasks = Vec::new();

    for (name, bs) in blockstates {
        match bs {
            Blockstate::Variants(vars) => {
                for props in vars.keys() {
                    tasks.push(ResolveTask::variant(name, props));
                }
            }
            Blockstate::Multipart(_) => tasks.push(ResolveTask::whole_block(name)),
        }
    }

    for (name, bs) in raw_blockstates {
        if blockstates.contains_key(name) {
            continue;
        }
        match bs {
            RawBlockstate::Variants(vars) => {
                for props in vars.keys() {
                    tasks.push(ResolveTask::variant(name, props));
                }
            }
            RawBlockstate::Multipart(_) => tasks.push(ResolveTask::whole_block(name)),
        }
    }

    for tex_key in textures.keys() {
        let Some(name) = synthesize_block_name(tex_key) else {
            continue;
        };
        if blockstates.contains_key(&name) || raw_blockstates.contains_key(&name) {
            continue;
        }
        tasks.push(ResolveTask::synthetic(name));
    }

    tasks
}

impl ResolveTask {
    fn variant(name: &str, props: &str) -> Self {
        let description = if props.is_empty() {
            name.to_string()
        } else {
            format!("{}|{}", name, props)
        };
        Self {
            name: name.to_string(),
            props: Some(props.to_string()),
            description,
            warn_on_failure: true,
        }
    }

    fn whole_block(name: &str) -> Self {
        Self {
            name: name.to_string(),
            props: None,
            description: name.to_string(),
            warn_on_failure: true,
        }
    }

    fn synthetic(name: String) -> Self {
        Self {
            description: name.clone(),
            name,
            props: None,
            warn_on_failure: false,
        }
    }
}

/// Convert a texture key like `"tfc:block/rock/raw/andesite"` to a candidate
/// block name `"tfc:rock/raw/andesite"`. Returns `None` for textures that
/// aren't under `block/` or `blocks/` (item icons, GUI sprites, etc.).
fn synthesize_block_name(tex_key: &str) -> Option<String> {
    let (ns, rest) = tex_key.split_once(':')?;
    let stripped = rest
        .strip_prefix("block/")
        .or_else(|| rest.strip_prefix("blocks/"))?;
    Some(format!("{}:{}", ns, stripped))
}
