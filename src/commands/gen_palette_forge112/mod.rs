// Palette generation for Forge 1.12.2 worlds (with RoughlyEnoughIDs / JEID).
//
// 1.12.2 mods ship proper `assets/<ns>/blockstates/` and `models/` JSONs, so
// we lean on the modern blockstate-aware resolver from `gen_palette` for
// modded blocks. Vanilla blocks reuse the hand-curated 1.7.10 (id, meta)
// table from `gen_palette_legacy::vanilla` — texture filenames under
// `assets/minecraft/textures/blocks/` are stable between 1.7.10 and 1.12.2,
// only the directory plural ("blocks/" vs "block/") shifted in 1.13.
//
// Pipeline:
//
//   1. Read `level.dat` → modern `FML.Registries.minecraft:blocks.ids`
//      compound → `{ numeric_id: "namespace:name" }`.
//   2. For each block:
//      - Vanilla in the curated table → emit one `id|meta` per variant
//        (probing both `block/` and `blocks/` texture-key forms).
//      - Otherwise → run the shared modern resolver and emit a single
//        bare-`id` color.
//   3. Apply biome tints (grass, leaves, vines) + special blocks
//      (water/lava/air) keyed by registered name.
//   4. Apply user overrides and write `format = "1.12.2"` palette JSON.
//
// See `docs/forge_1_12_2_rei.md` for the on-disk REI format spec.

mod leveldat;

use clap::Args;
use fastanvil::tex::{Blockstate, Renderer};
use log::{debug, info};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use crate::anvil::legacy::palette::{LegacyPaletteFile, Rgba};
use crate::commands::gen_palette::color::avg_colour;
use crate::commands::gen_palette::packs::load_packs;
use crate::commands::gen_palette::resolve::{Counters, Resolver, default_regex_mappings};
use crate::commands::gen_palette_legacy::vanilla;

use leveldat::load_fml_registry;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteForge112Args {
    /// Path to the world's `level.dat`. The modern FML block registry inside
    /// it (`FML.Registries.minecraft:blocks.ids`) defines the numeric block
    /// id ↔ name mapping the palette is keyed by.
    #[arg(short, long)]
    level_dat: PathBuf,

    /// Resource pack: a .jar/.zip file or a directory containing them.
    /// Repeatable — first-listed wins on conflicts (custom packs first,
    /// vanilla last). Mods are loaded the same way as resource packs;
    /// blockstate / model JSONs inside the jar are honored.
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

#[derive(Default, Debug)]
struct Stats {
    vanilla_table: usize,
    modded_resolved: usize,
    fallback: usize,
    skipped: usize,
}

const FALLBACK_GRAY: Rgba = [128, 128, 128, 255];

pub fn execute(args: GenPaletteForge112Args) -> Result<()> {
    info!("Starting palette generation (Forge 1.12.2 / REI)");
    info!("Level.dat: {}", args.level_dat.display());
    info!("Packs ({}):", args.pack.len());
    for p in &args.pack {
        info!("  - {}", p.display());
    }
    if let Some(ref o) = args.overrides {
        info!("Overrides: {}", o.display());
    }
    info!("Output: {}", args.output.display());

    let registry = load_fml_registry(&args.level_dat)?;
    info!("FML registry: {} blocks", registry.blocks.len());

    let pools = load_packs(&args.pack)?;
    let mut renderer = Renderer::new(
        pools.blockstates.clone(),
        pools.models.clone(),
        pools.textures.clone(),
    );
    let mappings = default_regex_mappings();
    let mut counters = Counters::default();
    let mut stats = Stats::default();

    let mut palette: HashMap<String, Rgba> = HashMap::new();
    // Track id → name so post-processing (biome tints, water/lava overrides)
    // can find the entries it needs to retint.
    let mut id_to_name: HashMap<u32, String> = HashMap::new();

    {
        let mut resolver = Resolver {
            renderer: &mut renderer,
            raw_blockstates: &pools.raw_blockstates,
            raw_models: &pools.raw_models,
            textures: &pools.textures,
            mappings: &mappings,
            counters: &mut counters,
        };

        for (id, name) in sort_by_id(&registry.blocks) {
            if name.is_empty() {
                stats.skipped += 1;
                continue;
            }
            id_to_name.insert(id, name.to_string());

            let (ns, local) = match name.split_once(':') {
                Some(p) => p,
                None => {
                    debug!("Skipping malformed block name: {}", name);
                    stats.skipped += 1;
                    continue;
                }
            };

            // Vanilla path: hand-curated (meta → texture) table that's known
            // to match 1.12.2 vanilla's getStateFromMeta output. Texture
            // lookups probe both `block/` and `blocks/` forms because the
            // table uses the 1.13+ `block/` convention but 1.12.2 vanilla's
            // raw paths are `blocks/`.
            if ns == "minecraft" {
                let variants = vanilla::variants_for(local);
                if !variants.is_empty() {
                    let mut first_color = None;
                    for (meta, tex_key) in variants {
                        if let Some(color) = lookup_vanilla_texture(&pools.textures, tex_key) {
                            palette.insert(format!("{}|{}", id, meta), color);
                            if first_color.is_none() {
                                first_color = Some(color);
                            }
                        }
                    }
                    if let Some(color) = first_color {
                        palette.insert(format!("{}", id), color);
                        stats.vanilla_table += 1;
                        continue;
                    }
                    // Table existed but no textures matched — fall through to
                    // the modern resolver as a safety net.
                }
            }

            // Modded (or vanilla-not-in-table) path: run the modern resolver
            // against the block's blockstate JSON. The blockstate may be
            // variants (multiple keys) or multipart; pick the first variant's
            // properties so the resolver can render an exact face.
            let probe_props = pools
                .blockstates
                .get(name)
                .and_then(|bs| match bs {
                    Blockstate::Variants(vars) => vars.keys().next().cloned(),
                    Blockstate::Multipart(_) => None,
                });
            if let Some(color) = resolver.resolve(name, probe_props.as_deref()) {
                palette.insert(format!("{}", id), color);
                stats.modded_resolved += 1;
            } else {
                palette.insert(format!("{}", id), FALLBACK_GRAY);
                stats.fallback += 1;
                debug!("No resolution for {} (id={})", name, id);
            }
        }
    }

    postprocess_vanilla(&mut palette, &id_to_name);

    if let Some(ref path) = args.overrides {
        let n = apply_overrides(&mut palette, path)?;
        info!("Applied {} override entries", n);
    }

    info!(
        "Resolved: {} vanilla-table, {} modded-resolved, {} fallback-gray, {} skipped",
        stats.vanilla_table, stats.modded_resolved, stats.fallback, stats.skipped
    );
    info!(
        "Resolver tiers: {} rendered, {} side, {} particle, {} any-tex, {} regex, {} probed, {} substring, {} generic-bs",
        counters.rendered,
        counters.side_fallback,
        counters.particle,
        counters.any_texture,
        counters.mapped,
        counters.probed,
        counters.substring,
        counters.generic_blockstate
    );

    let file = LegacyPaletteFile {
        format: "1.12.2".to_string(),
        blocks: palette,
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    std::fs::write(&args.output, &bytes)?;
    info!(
        "\u{2713} Forge 1.12.2 palette generation complete — {} entries written to {}",
        file.blocks.len(),
        args.output.display()
    );

    Ok(())
}

fn sort_by_id(blocks: &HashMap<u32, String>) -> Vec<(u32, &str)> {
    let mut v: Vec<(u32, &str)> = blocks.iter().map(|(k, v)| (*k, v.as_str())).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}

/// Try a texture key in both 1.13+ (`block/`) and 1.12.2-vanilla (`blocks/`)
/// forms. The vanilla 1.7.10 table this codebase already maintains uses the
/// `block/` form, but the modern packs loader keeps texture entries' paths
/// raw — so a vanilla 1.12.2 jar exposes them under `blocks/`. Probing both
/// avoids needing two parallel tables.
fn lookup_vanilla_texture(
    textures: &HashMap<String, fastanvil::tex::Texture>,
    key: &str,
) -> Option<Rgba> {
    if let Some(tex) = textures.get(key) {
        return Some(avg_colour(tex));
    }
    if let Some(alt) = swap_block_blocks(key) {
        if let Some(tex) = textures.get(&alt) {
            return Some(avg_colour(tex));
        }
    }
    None
}

/// Swap `:block/` ↔ `:blocks/` in a texture key. Returns None if neither
/// pattern is present.
fn swap_block_blocks(key: &str) -> Option<String> {
    if let Some(idx) = key.find(":block/") {
        let mut s = String::with_capacity(key.len() + 1);
        s.push_str(&key[..idx]);
        s.push_str(":blocks/");
        s.push_str(&key[idx + ":block/".len()..]);
        Some(s)
    } else if let Some(idx) = key.find(":blocks/") {
        let mut s = String::with_capacity(key.len());
        s.push_str(&key[..idx]);
        s.push_str(":block/");
        s.push_str(&key[idx + ":blocks/".len()..]);
        Some(s)
    } else {
        None
    }
}

fn postprocess_vanilla(palette: &mut HashMap<String, Rgba>, id_to_name: &HashMap<u32, String>) {
    let grass_tint: Rgba = [124, 189, 107, 255];
    let foliage_tint: Rgba = [84, 130, 54, 255];
    let vine_tint: Rgba = [106, 136, 44, 200];

    for (id, name) in id_to_name {
        let (_ns, local) = match name.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        match local {
            "air" => set_block_color(palette, *id, [0, 0, 0, 0]),
            "water" | "flowing_water" => set_block_color(palette, *id, [63, 118, 228, 180]),
            "lava" | "flowing_lava" => set_block_color(palette, *id, [207, 78, 0, 255]),
            "grass" | "mycelium" => multiply_block_color(palette, *id, grass_tint),
            "tallgrass" | "fern" | "double_plant" => {
                multiply_block_color(palette, *id, grass_tint)
            }
            "leaves" | "leaves2" | "waterlily" => {
                multiply_block_color(palette, *id, foliage_tint)
            }
            "vine" => multiply_block_color(palette, *id, vine_tint),
            _ => {}
        }
    }
}

fn set_block_color(palette: &mut HashMap<String, Rgba>, id: u32, color: Rgba) {
    let prefix_eq = format!("{}", id);
    let prefix_pipe = format!("{}|", id);
    let mut matching: Vec<String> = palette
        .keys()
        .filter(|k| *k == &prefix_eq || k.starts_with(&prefix_pipe))
        .cloned()
        .collect();
    matching.push(prefix_eq);
    for k in matching {
        palette.insert(k, color);
    }
}

fn multiply_block_color(palette: &mut HashMap<String, Rgba>, id: u32, tint: Rgba) {
    let prefix_eq = format!("{}", id);
    let prefix_pipe = format!("{}|", id);
    let keys: Vec<String> = palette
        .keys()
        .filter(|k| *k == &prefix_eq || k.starts_with(&prefix_pipe))
        .cloned()
        .collect();
    for k in keys {
        if let Some(existing) = palette.get(&k).copied() {
            palette.insert(k, multiply_rgba(existing, tint));
        }
    }
}

fn multiply_rgba(a: Rgba, b: Rgba) -> Rgba {
    [
        mul_channel(a[0], b[0]),
        mul_channel(a[1], b[1]),
        mul_channel(a[2], b[2]),
        mul_channel(a[3], b[3]),
    ]
}

#[inline]
fn mul_channel(a: u8, b: u8) -> u8 {
    (((a as u16) * (b as u16)) / 255) as u8
}

fn apply_overrides(
    palette: &mut HashMap<String, Rgba>,
    path: &std::path::Path,
) -> Result<usize> {
    let bytes = std::fs::read(path)?;
    let overrides: HashMap<String, [u8; 4]> = serde_json::from_slice(&bytes)?;
    let n = overrides.len();
    for (k, v) in overrides {
        palette.insert(k, v);
    }
    Ok(n)
}
