// Palette generation for pre-1.13 worlds (1.7.10, optionally with NEID).
//
// The modern gen-palette walks blockstate/model/texture JSONs. 1.7.10 has
// none of those — block rendering is all hard-coded in Java. Instead we:
//
//   1. Parse `level.dat` → FML `ItemData` registry → `{id: "ns:name"}`.
//   2. For each block, locate a reasonable texture in the supplied jars:
//      - Vanilla (`minecraft:`) uses a hand-curated `(name, meta) → texture`
//        table (see `vanilla.rs`); 1.7.10 texture layout is stable and small
//        enough that this is tractable.
//      - Modded blocks fall back to filename-based matching in the mod's
//        `assets/<ns>/textures/blocks/` tree.
//   3. Average each texture into a color using the same logic as the modern
//      path (`gen_palette::color::avg_colour`).
//   4. Emit a JSON palette wrapped with `"format": "1.7.10"` so the renderer
//      can distinguish it from the modern flat-map palette.

mod leveldat;
mod packs;
mod resolve;
mod vanilla;

use clap::Args;
use log::{debug, info};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use crate::anvil::legacy::palette::{LegacyPaletteFile, Rgba};
use crate::commands::gen_palette::color::avg_colour;

use leveldat::{FmlRegistry, load_fml_registry};
use packs::{TexturePack, load_texture_packs};
use resolve::{MatchKind, ResolveStats, resolve_modded};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteLegacyArgs {
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

pub fn execute(args: GenPaletteLegacyArgs) -> Result<()> {
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

    let packs = load_texture_packs(&args.pack)?;
    let total_textures: usize = packs.iter().map(|p| p.textures.len()).sum();
    info!(
        "Loaded {} texture pack(s), {} total textures",
        packs.len(),
        total_textures
    );

    let mut palette: HashMap<String, Rgba> = HashMap::new();
    let mut stats = ResolveStats::default();

    // Track which (numeric id) keys came from which `namespace:name` so we
    // can apply name-driven post-processing (biome tints, water/lava fixups)
    // at the end.
    let mut id_to_name: HashMap<u16, String> = HashMap::new();

    // Per-block: try vanilla table first (covers meta variants), then modded
    // fallback. Emit an `id|meta` key per resolved meta, plus a bare `id` as
    // the fallback for unknown metas.
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

    // Vanilla biome tints + water/lava overrides. Runs before user overrides
    // so the user can still override them.
    postprocess_vanilla(&mut palette, &id_to_name);

    // User overrides win over everything.
    if let Some(ref path) = args.overrides {
        let n = apply_overrides(&mut palette, path)?;
        info!("Applied {} override entries", n);
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
    info!(
        "\u{2713} Legacy palette generation complete — {} entries written to {}",
        file.blocks.len(),
        args.output.display()
    );

    Ok(())
}

/// Sort blocks by numeric ID for a stable, human-readable output order.
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
    let variants = vanilla::variants_for(local);
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

const FALLBACK_GRAY: Rgba = [128, 128, 128, 255];

/// Apply static post-processing to vanilla-namespaced blocks. Required for
/// grass/leaves/vines (grayscale textures that Minecraft tints per-biome at
/// runtime) and for water/lava/air (special values the renderer treats
/// specially).
fn postprocess_vanilla(palette: &mut HashMap<String, Rgba>, id_to_name: &HashMap<u16, String>) {
    // Tints applied to every `id|meta` + bare `id` entry of the block.
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
            "water" | "flowing_water" => {
                set_block_color(palette, *id, [63, 118, 228, 180])
            }
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

fn set_block_color(palette: &mut HashMap<String, Rgba>, id: u16, color: Rgba) {
    let prefix = format!("{}", id);
    let mut matching: Vec<String> = palette
        .keys()
        .filter(|k| *k == &prefix || k.starts_with(&format!("{}|", id)))
        .cloned()
        .collect();
    matching.push(prefix);
    for k in matching {
        palette.insert(k, color);
    }
}

fn multiply_block_color(palette: &mut HashMap<String, Rgba>, id: u16, tint: Rgba) {
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

fn lookup_texture(packs: &[TexturePack], key: &str) -> Option<Rgba> {
    for p in packs {
        if let Some(tex) = p.textures.get(key) {
            return Some(avg_colour(tex));
        }
    }
    None
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
