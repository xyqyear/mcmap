// Generate palette from Minecraft / mod jar resource packs.
//
// Treats vanilla and modded jars identically: every pack is a zip archive
// containing `assets/<namespace>/{blockstates,models,textures}/...`. The
// namespace is derived from the path, never hardcoded.
//
// Resolution tiers (first success wins):
//   0. fastanvil renderer — blockstate variant → model → top face texture.
//   1. raw-model side-face fallback — any face ('up','down','north',...) from
//      the variant's model, or from any other variant of the same block,
//      or the first `apply` model of a multipart blockstate.
//   2. regex rewrites — namespace-agnostic for generic patterns
//      (fences→planks, walls→planks), minecraft-specific for vanilla quirks
//      (crops at final stage, fire_0, bamboo_stalk, etc.).
//   3. texture-path probe — direct lookup at `<ns>:block/<name>`.
//   4. user overrides JSON (`--overrides`) — final authoritative precedence,
//      applied after all automatic resolution.

pub(crate) mod color;
pub(crate) mod packs;
mod postprocess;
pub(crate) mod raw;
pub(crate) mod resolve;

use clap::Args;
use fastanvil::{
    Rgba,
    tex::{Blockstate, Renderer},
};
use log::{info, warn};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use packs::{Pools, load_packs};
use postprocess::{add_base_colors, add_missing_blocks, load_overrides};
use resolve::{Counters, Resolver, default_regex_mappings};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteArgs {
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

pub fn execute(args: GenPaletteArgs) -> Result<()> {
    info!("Starting palette generation");
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
    // Capture blockstate names before mutably borrowing renderer.
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
                    // fastanvil can't render multipart; go straight to
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

    // User overrides take final precedence over everything automatic.
    if let Some(ref path) = args.overrides {
        info!("Applying overrides from {}", path.display());
        let overrides = load_overrides(path)?;
        let n = overrides.len();
        for (k, v) in overrides {
            palette.insert(k, v);
        }
        info!("  Applied {} override entries", n);
    }

    info!("Writing palette to: {}", args.output.display());
    let file = std::fs::File::create(&args.output)?;
    serde_json::to_writer_pretty(file, &palette)?;

    info!(
        "\u{2713} Palette generation complete! {} total blocks written",
        palette.len()
    );

    Ok(())
}
