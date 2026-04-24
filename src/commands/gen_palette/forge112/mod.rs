// `gen-palette forge112` — palette generation for Forge 1.12.2 worlds running
// RoughlyEnoughIDs / JustEnoughIDs.
//
// 1.12.2 mods ship proper `assets/<ns>/blockstates/` and `models/` JSONs, so
// modded block resolution reuses the modern blockstate-aware resolver from
// `modern_pack`. Vanilla blocks reuse the hand-curated 1.7.10 (name, meta)
// table from `shared::vanilla_1x` — texture filenames under
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

mod leveldat;

use clap::Args;
use fastanvil::tex::{Blockstate, Renderer};
use log::{debug, info};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use super::modern_pack::{Counters, Resolver, default_regex_mappings, load_packs};
use super::shared::color::avg_colour;
use super::shared::overrides::{apply_overrides, load_overrides};
use super::shared::progress::PackLoadReport;
use super::shared::vanilla_1x;
use crate::anvil::legacy::palette::LegacyPaletteFile;
use crate::anvil::palette::Rgba;
use crate::output::emit_if_json;
use leveldat::load_fml_registry;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct Forge112Args {
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

#[derive(Serialize)]
struct RegistryLoadedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    blocks: usize,
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

#[derive(Serialize, Clone, Copy)]
struct ClassificationCounters {
    vanilla_table: usize,
    modded_resolved: usize,
    fallback_gray: usize,
    skipped: usize,
}

impl From<&Stats> for ClassificationCounters {
    fn from(s: &Stats) -> Self {
        Self {
            vanilla_table: s.vanilla_table,
            modded_resolved: s.modded_resolved,
            fallback_gray: s.fallback,
            skipped: s.skipped,
        }
    }
}

#[derive(Serialize, Clone, Copy)]
struct Forge112Counters {
    classification: ClassificationCounters,
    resolver: ResolverCounters,
}

#[derive(Serialize)]
struct ResolvedEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    counters: Forge112Counters,
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
    counters: Forge112Counters,
}

pub fn execute(args: Forge112Args) -> Result<()> {
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
    emit_if_json(&RegistryLoadedEvent {
        ty: "progress",
        phase: "registry_loaded",
        blocks: registry.blocks.len(),
    });

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

            // Vanilla path: hand-curated (meta → texture) table known to
            // match 1.12.2 vanilla's getStateFromMeta. Texture lookups probe
            // both `block/` and `blocks/` forms because the table uses the
            // 1.13+ `block/` convention but 1.12.2 vanilla's raw paths are
            // `blocks/`.
            if ns == "minecraft" {
                let variants = vanilla_1x::variants_for(local);
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

            // Modded (or vanilla-not-in-table): run the modern resolver
            // against the block's blockstate JSON. Pick the first variant's
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

    vanilla_1x::apply_vanilla_postprocess(&mut palette, &id_to_name);

    let forge_counters = Forge112Counters {
        classification: ClassificationCounters::from(&stats),
        resolver: ResolverCounters::from(&counters),
    };
    emit_if_json(&ResolvedEvent {
        ty: "progress",
        phase: "resolved",
        counters: forge_counters,
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
        "Forge 1.12.2 palette generation complete — {} entries written to {}",
        file.blocks.len(),
        args.output.display()
    );
    emit_if_json(&ResultEvent {
        ty: "result",
        output: args.output.display().to_string(),
        entries: file.blocks.len(),
        counters: forge_counters,
    });

    Ok(())
}

fn sort_by_id(blocks: &HashMap<u32, String>) -> Vec<(u32, &str)> {
    let mut v: Vec<(u32, &str)> = blocks.iter().map(|(k, v)| (*k, v.as_str())).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}

/// Try a texture key in both 1.13+ (`block/`) and 1.12.2-vanilla (`blocks/`)
/// forms. The vanilla 1.7.10 table this codebase shares uses the `block/`
/// form, but the modern packs loader keeps texture entries' paths raw — so
/// a vanilla 1.12.2 jar exposes them under `blocks/`. Probing both avoids
/// needing two parallel tables.
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
