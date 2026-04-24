// `gen-palette modern` — palette generation for 1.13+ worlds.
//
// Walks standard blockstate/model/texture JSONs across every supplied resource
// pack (vanilla and modded are treated identically; namespace is derived from
// the on-disk path). Resolution tiers are listed at the top of
// `modern_pack/mod.rs`.

use clap::Args;
use fastanvil::{
    Rgba,
    tex::{Blockstate, Renderer},
};
use log::{info, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use super::modern_pack::{
    Counters, Pools, Resolver, add_base_colors, add_missing_blocks, default_regex_mappings,
    load_packs,
};
use super::shared::overrides::{apply_overrides, load_overrides};
use super::shared::progress::PackLoadReport;
use crate::output::emit_if_json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct ModernArgs {
    /// Resource pack to load: a .jar/.zip file, or a directory containing
    /// .jar/.zip files at depth 1. Repeatable; first-listed wins on conflict
    /// (list user packs first, vanilla last).
    #[arg(short, long, required = true)]
    pack: Vec<PathBuf>,

    /// Output palette.json file path
    #[arg(short, long, default_value = "palette.json")]
    output: PathBuf,

    /// Optional user overrides file. JSON map of `"namespace:id"` →
    /// `[r,g,b,a]`. Applied last — overrides everything automatic.
    #[arg(long)]
    overrides: Option<PathBuf>,
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

pub fn execute(args: ModernArgs) -> Result<()> {
    info!("Starting palette generation (modern / 1.13+)");
    info!("Packs ({}):", args.pack.len());
    for p in &args.pack {
        info!("  - {}", p.display());
    }
    if let Some(ref o) = args.overrides {
        info!("Overrides: {}", o.display());
    }
    info!("Output: {}", args.output.display());

    let pools = load_packs(&args.pack, |report: &PackLoadReport| {
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
        pack_count: args.pack.len(),
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
    let bs_names: Vec<String> = blockstates.keys().cloned().collect();
    {
        let mut resolver = Resolver {
            renderer: &mut renderer,
            raw_blockstates: &raw_blockstates,
            raw_models: &raw_models,
            textures: &textures,
            mappings: &mappings,
            counters: &mut counters,
        };
        for name in &bs_names {
            let bs = &blockstates[name];
            match bs {
                Blockstate::Variants(vars) => {
                    for props in vars.keys() {
                        let description =
                            name.clone() + if props.is_empty() { "" } else { "|" } + props;
                        if let Some(col) = resolver.resolve(name, Some(props)) {
                            palette.insert(description, col);
                        } else {
                            warn!("Could not resolve: {}", description);
                            failed += 1;
                        }
                    }
                }
                Blockstate::Multipart(_) => {
                    // fastanvil can't render multipart; go straight to the
                    // raw-model fallback (tier 1+) via props=None.
                    if let Some(col) = resolver.resolve(name, None) {
                        palette.insert(name.clone(), col);
                    } else {
                        warn!("Could not resolve multipart: {}", name);
                        failed += 1;
                    }
                }
            }
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

    if let Some(ref path) = args.overrides {
        info!("Applying overrides from {}", path.display());
        let overrides = load_overrides(path)?;
        let n = apply_overrides(&mut palette, overrides);
        info!("  Applied {} override entries", n);
        emit_if_json(&OverridesAppliedEvent {
            ty: "progress",
            phase: "overrides_applied",
            count: n,
        });
    }

    info!("Writing palette to: {}", args.output.display());
    let file = std::fs::File::create(&args.output)?;
    serde_json::to_writer_pretty(file, &palette)?;

    info!(
        "Palette generation complete — {} total blocks written",
        palette.len()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        output: args.output.display().to_string(),
        entries: palette.len(),
        failed,
        counters: resolver_counters,
    });
    Ok(())
}
