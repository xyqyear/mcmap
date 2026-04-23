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
use std::collections::HashMap;
use std::path::PathBuf;

use super::modern_pack::{
    Counters, Pools, Resolver, add_base_colors, add_missing_blocks, default_regex_mappings,
    load_packs,
};
use super::shared::overrides::{apply_overrides, load_overrides};

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

    let pools = load_packs(&args.pack)?;
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
    }

    info!("Writing palette to: {}", args.output.display());
    let file = std::fs::File::create(&args.output)?;
    serde_json::to_writer_pretty(file, &palette)?;

    info!(
        "Palette generation complete — {} total blocks written",
        palette.len()
    );
    Ok(())
}
