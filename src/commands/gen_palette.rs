// Generate palette from Minecraft / mod jar resource packs.
//
// Treats vanilla and modded jars identically: every pack is a zip archive
// containing `assets/<namespace>/{blockstates,models,textures}/...`. The
// namespace is derived from the path, never hardcoded.

use clap::Args;
use fastanvil::{
    tex::{Blockstate, Model, Render, Renderer, Texture},
    Rgba,
};
use log::{debug, error, info, warn};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

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
}

fn avg_colour(rgba_data: &[u8]) -> Rgba {
    let mut avg = [0f64; 4];
    let mut count = 0;

    for p in rgba_data.chunks(4) {
        avg[0] += ((p[0] as u64) * (p[0] as u64)) as f64;
        avg[1] += ((p[1] as u64) * (p[1] as u64)) as f64;
        avg[2] += ((p[2] as u64) * (p[2] as u64)) as f64;
        avg[3] += ((p[3] as u64) * (p[3] as u64)) as f64;
        count += 1;
    }

    [
        (avg[0] / count as f64).sqrt() as u8,
        (avg[1] / count as f64).sqrt() as u8,
        (avg[2] / count as f64).sqrt() as u8,
        (avg[3] / count as f64).sqrt() as u8,
    ]
}

#[derive(Default)]
struct Pools {
    blockstates: HashMap<String, Blockstate>,
    models: HashMap<String, Model>,
    textures: HashMap<String, Texture>,
}

#[derive(Copy, Clone, Debug)]
enum Category {
    Blockstate,
    Model,
    Texture,
}

/// Parse a zip entry path like `assets/<ns>/<category>/<rest>.<ext>` into a
/// (category, "namespace:rest_without_ext") key. Returns None for any entry
/// that isn't a supported resource.
fn parse_entry(entry_name: &str) -> Option<(Category, String)> {
    let s = entry_name.trim_start_matches('/').trim_start_matches("./");
    let mut parts = s.splitn(4, '/');
    let assets = parts.next()?;
    if assets != "assets" {
        return None;
    }
    let namespace = parts.next()?;
    let category_str = parts.next()?;
    let rest = parts.next()?;
    if namespace.is_empty() || rest.is_empty() {
        return None;
    }

    let category = match category_str {
        "blockstates" => Category::Blockstate,
        "models" => Category::Model,
        "textures" => Category::Texture,
        _ => return None,
    };

    let (rest_no_ext, ext) = rest.rsplit_once('.')?;
    let ext_ok = match category {
        Category::Blockstate | Category::Model => ext.eq_ignore_ascii_case("json"),
        Category::Texture => ext.eq_ignore_ascii_case("png"),
    };
    if !ext_ok {
        return None;
    }

    Some((category, format!("{}:{}", namespace, rest_no_ext)))
}

/// Expand input paths into an ordered list of archive files.
/// User-supplied argument order is preserved; within a directory, entries are
/// sorted alphabetically for determinism.
fn expand_packs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for p in paths {
        if !p.exists() {
            return Err(format!("Pack path not found: {}", p.display()).into());
        }
        if p.is_file() {
            expanded.push(p.clone());
            continue;
        }
        if p.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(p)?
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .filter(|e| e.is_file())
                .filter(|e| {
                    matches!(
                        e.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()),
                        Some(ref s) if s == "jar" || s == "zip"
                    )
                })
                .collect();
            entries.sort();
            if entries.is_empty() {
                warn!("No .jar/.zip files found in directory: {}", p.display());
            }
            expanded.extend(entries);
        } else {
            return Err(format!("Pack path is neither file nor directory: {}", p.display()).into());
        }
    }
    Ok(expanded)
}

/// Read one archive, inserting every resource that isn't already present in
/// the pools. First-wins semantics.
fn load_archive(path: &Path, pools: &mut Pools) -> Result<()> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut bs_added = 0usize;
    let mut m_added = 0usize;
    let mut t_added = 0usize;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let Some((category, key)) = parse_entry(&name) else {
            continue;
        };

        match category {
            Category::Blockstate => {
                if pools.blockstates.contains_key(&key) {
                    continue;
                }
                let mut buf = Vec::new();
                if let Err(e) = entry.read_to_end(&mut buf) {
                    debug!("Failed to read {}: {}", name, e);
                    continue;
                }
                match serde_json::from_slice::<Blockstate>(&buf) {
                    Ok(bs) => {
                        pools.blockstates.insert(key, bs);
                        bs_added += 1;
                    }
                    Err(e) => debug!("Failed to parse blockstate {}: {}", name, e),
                }
            }
            Category::Model => {
                if pools.models.contains_key(&key) {
                    continue;
                }
                let mut buf = Vec::new();
                if let Err(e) = entry.read_to_end(&mut buf) {
                    debug!("Failed to read {}: {}", name, e);
                    continue;
                }
                match serde_json::from_slice::<Model>(&buf) {
                    Ok(m) => {
                        pools.models.insert(key, m);
                        m_added += 1;
                    }
                    Err(e) => debug!("Failed to parse model {}: {}", name, e),
                }
            }
            Category::Texture => {
                if pools.textures.contains_key(&key) {
                    continue;
                }
                let mut buf = Vec::new();
                if let Err(e) = entry.read_to_end(&mut buf) {
                    debug!("Failed to read {}: {}", name, e);
                    continue;
                }
                match image::load_from_memory(&buf) {
                    Ok(img) => {
                        pools.textures.insert(key, img.to_rgba8().into_raw());
                        t_added += 1;
                    }
                    Err(e) => debug!("Failed to decode texture {}: {}", name, e),
                }
            }
        }
    }

    info!(
        "  + {} blockstates, {} models, {} textures",
        bs_added, m_added, t_added
    );
    Ok(())
}

fn load_packs(paths: &[PathBuf]) -> Result<Pools> {
    let archives = expand_packs(paths)?;
    if archives.is_empty() {
        return Err("No pack files to load (did you pass empty directories?)".into());
    }

    let mut pools = Pools::default();
    for archive_path in &archives {
        info!("Loading pack: {}", archive_path.display());
        if let Err(e) = load_archive(archive_path, &mut pools) {
            error!("Failed to load {}: {}", archive_path.display(), e);
        }
    }

    info!(
        "Totals: {} blockstates, {} models, {} textures across {} pack(s)",
        pools.blockstates.len(),
        pools.models.len(),
        pools.textures.len(),
        archives.len()
    );
    Ok(pools)
}

#[derive(Debug)]
struct RegexMapping {
    blockstate: Regex,
    texture_template: &'static str,
}

impl RegexMapping {
    fn apply(&self, blockstate: &str) -> Option<String> {
        let caps = self.blockstate.captures(blockstate)?;
        let mut i = 1;
        let mut tex = self.texture_template.to_string();

        for cap in caps.iter().skip(1) {
            let cap = match cap {
                Some(cap) => cap,
                None => continue,
            };
            tex = tex.replace(&format!("${}", i), cap.into());
            i += 1;
        }

        Some(tex)
    }
}

/// Vanilla-only fallbacks for blocks the renderer can't derive a color for
/// (water, lava, air, etc.). Mods that need custom fallbacks can't express
/// that here today; they'd need to ship a renderable model.
fn add_missing_blocks(palette: &mut HashMap<String, Rgba>) {
    info!("Adding missing common blocks");

    let missing = vec![
        ("minecraft:air", [0, 0, 0, 0]),
        ("minecraft:water", [63, 118, 228, 180]),
        ("minecraft:flowing_water", [63, 118, 228, 180]),
        ("minecraft:lava", [207, 78, 0, 255]),
        ("minecraft:flowing_lava", [207, 78, 0, 255]),
        ("minecraft:vine", [106, 136, 44, 200]),
        ("minecraft:grass", [124, 189, 107, 255]),
        ("minecraft:fern", [104, 149, 92, 255]),
    ];

    for (name, color) in missing {
        if !palette.contains_key(name) {
            palette.insert(name.to_string(), color);
            info!("  Added missing block: {}", name);
        }
    }
}

/// Adds an unqualified `<ns>:<name>` entry for blocks that only have
/// `<ns>:<name>|<state>` variants, for O(1) lookup fallback. Namespace-agnostic.
fn add_base_colors(palette: &mut HashMap<String, Rgba>) {
    info!("Adding base colors for state variants");

    let mut blocks_with_states: HashMap<String, Vec<Rgba>> = HashMap::new();
    let mut blocks_without_states = std::collections::HashSet::new();

    for (key, &color) in palette.iter() {
        if key.contains('|') {
            let base_name = key.split('|').next().unwrap().to_string();
            blocks_with_states.entry(base_name).or_default().push(color);
        } else {
            blocks_without_states.insert(key.clone());
        }
    }

    let mut added = 0;
    for (base_name, colors) in blocks_with_states {
        if !blocks_without_states.contains(&base_name) {
            palette.insert(base_name.clone(), colors[0]);
            added += 1;
        }
    }

    info!("  Added {} base block colors", added);
}

pub fn execute(args: GenPaletteArgs) -> Result<()> {
    info!("Starting palette generation");
    info!("Packs ({}):", args.pack.len());
    for p in &args.pack {
        info!("  - {}", p.display());
    }
    info!("Output: {}", args.output.display());

    let pools = load_packs(&args.pack)?;
    let Pools {
        blockstates,
        models,
        textures,
    } = pools;

    info!("Creating renderer");
    let mut renderer = Renderer::new(blockstates.clone(), models, textures.clone());
    let mut failed = 0;
    let mut mapped = 0;
    let mut success = 0;

    // Vanilla-specific fallbacks: if the renderer can't handle a blockstate,
    // try these regex→texture rewrites. The `minecraft:` prefix here is
    // intentional — these are patches for known vanilla quirks. Mod blocks
    // with similar quirks would need their own entries.
    let mappings = vec![
        RegexMapping {
            blockstate: Regex::new(r"minecraft:(.+)_fence").unwrap(),
            texture_template: "minecraft:block/$1_planks",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:(.+)_wall(_sign)?").unwrap(),
            texture_template: "minecraft:block/$1_planks",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:(.+)_wall(_sign)?").unwrap(),
            texture_template: "minecraft:block/$1",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:wheat").unwrap(),
            texture_template: "minecraft:block/wheat_stage7",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:carrots").unwrap(),
            texture_template: "minecraft:block/carrots_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:lava").unwrap(),
            texture_template: "minecraft:block/lava_still",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:sugar_cane").unwrap(),
            texture_template: "minecraft:block/sugar_cane",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:fire").unwrap(),
            texture_template: "minecraft:block/fire_0",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:potatoes").unwrap(),
            texture_template: "minecraft:block/potatoes_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:beetroots").unwrap(),
            texture_template: "minecraft:block/beetroots_stage3",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:tripwire").unwrap(),
            texture_template: "minecraft:block/tripwire",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:bamboo").unwrap(),
            texture_template: "minecraft:block/bamboo_stalk",
        },
        RegexMapping {
            blockstate: Regex::new(r"minecraft:sweet_berry_bush").unwrap(),
            texture_template: "minecraft:block/sweet_berry_bush_stage3",
        },
    ];

    let mut palette = HashMap::new();

    let mut try_mapping = |mapping: &RegexMapping, blockstate: String| {
        if let Some(tex) = mapping.apply(&blockstate) {
            if let Some(texture) = textures.get(&tex) {
                info!("Mapped {} to {}", blockstate, tex);
                mapped += 1;
                return Some(avg_colour(texture.as_slice()));
            }
        }
        None
    };

    let mut try_mappings = |blockstate: String| {
        mappings
            .iter()
            .map(|mapping| try_mapping(mapping, blockstate.clone()))
            .find_map(|col| col)
            .or_else(|| {
                warn!("Could not map: {}", blockstate);
                failed += 1;
                None
            })
    };

    info!("Rendering blockstates");
    for name in blockstates.keys() {
        let bs = &blockstates[name];

        match bs {
            Blockstate::Variants(vars) => {
                for props in vars.keys() {
                    let res = renderer.get_top(name, props);
                    match res {
                        Ok(texture) => {
                            let col = avg_colour(texture.as_slice());
                            let description =
                                (*name).clone() + if props.is_empty() { "" } else { "|" } + props;
                            palette.insert(description, col);
                            success += 1;
                        }
                        Err(_) => {
                            if let Some(c) = try_mappings((*name).clone()) {
                                palette.insert((*name).clone(), c);
                            }
                        }
                    };
                }
            }
            Blockstate::Multipart(_) => {
                if let Some(c) = try_mappings((*name).clone()) {
                    palette.insert((*name).clone(), c);
                }
            }
        }
    }

    info!(
        "Rendered {} successful, {} mapped, {} failed",
        success, mapped, failed
    );

    // 1.17 renamed grass_path to dirt_path. Keep the old name working.
    if let Some(path) = palette.get("minecraft:dirt_path").cloned() {
        palette.insert("minecraft:grass_path".into(), path);
    }

    add_missing_blocks(&mut palette);
    add_base_colors(&mut palette);

    info!("Writing palette to: {}", args.output.display());
    let file = std::fs::File::create(&args.output)?;
    serde_json::to_writer_pretty(file, &palette)?;

    info!(
        "\u{2713} Palette generation complete! {} total blocks written",
        palette.len()
    );

    Ok(())
}
