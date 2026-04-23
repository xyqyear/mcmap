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

mod color;
mod packs;
mod postprocess;
mod raw;
mod resolve;

use clap::Args;
use fastanvil::{
    Rgba,
    tex::{Blockstate, Render, Renderer},
};
use log::{debug, info, warn};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use color::avg_colour;
use packs::{Pools, load_packs};
use postprocess::{add_base_colors, add_missing_blocks, load_overrides};
use resolve::{
    Counters, RegexMapping, probe_texture_by_name, render_any_face, render_any_texture_ref,
    render_any_variant_of_block, render_particle_texture,
};

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

    // Regex rewrites for block IDs the renderer can't resolve.
    //
    // Generic patterns use `([^:]+):(.+)` so they apply to any namespace —
    // wood-like naming conventions are widely reused by mods. Vanilla-specific
    // patterns (hardcoded stage numbers, special-case frame 0) stay minecraft:
    // since they mirror quirks of the vanilla asset layout only.
    let mappings = vec![
        // Generic (namespace-agnostic)
        RegexMapping {
            blockstate: Regex::new(r"([^:]+):(.+)_fence$").unwrap(),
            texture_template: "$1:block/$2_planks",
        },
        RegexMapping {
            blockstate: Regex::new(r"([^:]+):(.+)_fence_gate$").unwrap(),
            texture_template: "$1:block/$2_planks",
        },
        RegexMapping {
            blockstate: Regex::new(r"([^:]+):(.+)_wall(_sign)?$").unwrap(),
            texture_template: "$1:block/$2_planks",
        },
        RegexMapping {
            blockstate: Regex::new(r"([^:]+):(.+)_wall(_sign)?$").unwrap(),
            texture_template: "$1:block/$2",
        },
        // Vanilla-only quirks (hardcoded stage numbers etc.)
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:wheat$").unwrap(),
            texture_template: "minecraft:block/wheat_stage7",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:carrots$").unwrap(),
            texture_template: "minecraft:block/carrots_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:lava$").unwrap(),
            texture_template: "minecraft:block/lava_still",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:sugar_cane$").unwrap(),
            texture_template: "minecraft:block/sugar_cane",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:fire$").unwrap(),
            texture_template: "minecraft:block/fire_0",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:potatoes$").unwrap(),
            texture_template: "minecraft:block/potatoes_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:beetroots$").unwrap(),
            texture_template: "minecraft:block/beetroots_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:tripwire$").unwrap(),
            texture_template: "minecraft:block/tripwire",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:bamboo$").unwrap(),
            texture_template: "minecraft:block/bamboo_stalk",
        },
        RegexMapping {
            blockstate: Regex::new(r"^minecraft:sweet_berry_bush$").unwrap(),
            texture_template: "minecraft:block/sweet_berry_bush_stage3",
        },
    ];

    let mut palette: HashMap<String, Rgba> = HashMap::new();

    // Tiered resolver: tries fastanvil → raw-model side-face → particle-only
    // → any-texture-ref (custom loaders) → regex rewrites → texture-path probe,
    // in that order. Returns None only if every tier fails.
    let try_resolve = |name: &str,
                       props: Option<&str>,
                       renderer: &mut Renderer,
                       c: &mut Counters|
     -> Option<Rgba> {
        // Tier 0: fastanvil renderer on the exact variant.
        if let Some(p) = props {
            if let Ok(tex) = renderer.get_top(name, p) {
                c.rendered += 1;
                return Some(avg_colour(&tex));
            }
        }
        if let Some(raw_bs) = raw_blockstates.get(name) {
            // Tier 1: any-face across variants / multipart parts.
            if let Some(tex) = render_any_variant_of_block(raw_bs, &raw_models, |m| {
                render_any_face(m, &textures)
            }) {
                c.side_fallback += 1;
                return Some(avg_colour(&tex));
            }
            // Tier 1.5: models without `elements` (signs, beds, chests,
            // banners, heads, Botania buried_petals/floating_*, …) still
            // declare a `particle` texture Mojang picks to match the break
            // particle. Use that as the block color.
            if let Some(tex) = render_any_variant_of_block(raw_bs, &raw_models, |m| {
                render_particle_texture(m, &textures)
            }) {
                c.particle += 1;
                return Some(avg_colour(&tex));
            }
            // Tier 1.7: Forge custom loaders (functionalstorage:framedblock,
            // framedblocks, etc.) skip `elements` entirely. Fall back to any
            // texture ref anywhere in the model + its children.
            if let Some(tex) = render_any_variant_of_block(raw_bs, &raw_models, |m| {
                render_any_texture_ref(m, &raw_models, &textures)
            }) {
                c.any_texture += 1;
                return Some(avg_colour(&tex));
            }
        }
        // Tier 2: regex rewrites (generic + vanilla quirks).
        for mapping in &mappings {
            if let Some(tex_name) = mapping.apply(name) {
                if let Some(tex) = textures.get(&tex_name) {
                    debug!("Regex mapped {} → {}", name, tex_name);
                    c.mapped += 1;
                    return Some(avg_colour(tex));
                }
            }
        }
        // Tier 3: direct texture-path probe by block name.
        if let Some(tex) = probe_texture_by_name(name, &textures) {
            debug!("Probed texture for {}", name);
            c.probed += 1;
            return Some(avg_colour(&tex));
        }
        None
    };

    info!("Rendering blockstates");
    // Capture blockstate names before mutably borrowing renderer.
    let bs_names: Vec<String> = blockstates.keys().cloned().collect();
    for name in &bs_names {
        let bs = &blockstates[name];

        match bs {
            Blockstate::Variants(vars) => {
                for props in vars.keys() {
                    let description =
                        name.clone() + if props.is_empty() { "" } else { "|" } + props;
                    if let Some(col) =
                        try_resolve(name, Some(props), &mut renderer, &mut counters)
                    {
                        palette.insert(description, col);
                    } else {
                        warn!("Could not resolve: {}", description);
                        failed += 1;
                    }
                }
            }
            Blockstate::Multipart(_) => {
                // fastanvil can't render multipart; go straight to raw-model
                // fallback (tier 1+) via `try_resolve` with props=None.
                if let Some(col) = try_resolve(name, None, &mut renderer, &mut counters) {
                    palette.insert(name.clone(), col);
                } else {
                    warn!("Could not resolve multipart: {}", name);
                    failed += 1;
                }
            }
        }
    }

    info!(
        "Resolved: {} rendered, {} side/multipart, {} particle, {} any-texture, {} regex, {} probed, {} failed",
        counters.rendered,
        counters.side_fallback,
        counters.particle,
        counters.any_texture,
        counters.mapped,
        counters.probed,
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
