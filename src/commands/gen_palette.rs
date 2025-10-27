// Generate palette from Minecraft JAR assets
// Simplified version of fastnbt-tools' anvil-palette

use clap::Args;
use fastanvil::{
    tex::{Blockstate, Model, Render, Renderer, Texture},
    Rgba,
};
use log::{error, info, warn};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteArgs {
    /// Path to extracted Minecraft JAR assets folder (e.g., minecraft/assets/minecraft)
    #[arg(short, long)]
    assets: PathBuf,

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

fn load_texture(path: &Path) -> Result<Texture> {
    let img = image::open(path)?;
    let img = img.to_rgba8();
    Ok(img.into_raw())
}

fn load_blockstates(blockstates_path: &Path) -> Result<HashMap<String, Blockstate>> {
    info!("Loading blockstates from: {}", blockstates_path.display());
    let mut blockstates = HashMap::<String, Blockstate>::new();

    for entry in std::fs::read_dir(blockstates_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let json = std::fs::read_to_string(&path)?;
            let json: Blockstate = serde_json::from_str(&json)?;
            blockstates.insert(
                "minecraft:".to_owned()
                    + path
                        .file_stem()
                        .ok_or(format!("invalid file name: {}", path.display()))?
                        .to_str()
                        .ok_or(format!("nonunicode file name: {}", path.display()))?,
                json,
            );
        }
    }

    info!("Loaded {} blockstates", blockstates.len());
    Ok(blockstates)
}

fn load_models(path: &Path) -> Result<HashMap<String, Model>> {
    info!("Loading models from: {}", path.display());
    let mut models = HashMap::<String, Model>::new();

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let json = std::fs::read_to_string(&path)?;
            let json: Model = serde_json::from_str(&json)?;
            models.insert(
                "minecraft:block/".to_owned()
                    + path
                        .file_stem()
                        .ok_or(format!("invalid file name: {}", path.display()))?
                        .to_str()
                        .ok_or(format!("nonunicode file name: {}", path.display()))?,
                json,
            );
        }
    }

    info!("Loaded {} models", models.len());
    Ok(models)
}

fn load_textures(path: &Path) -> Result<HashMap<String, Texture>> {
    info!("Loading textures from: {}", path.display());
    let mut tex = HashMap::new();

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().ok_or("invalid ext")?.to_string_lossy() == "png" {
            let texture = load_texture(&path);

            match texture {
                Err(_) => continue,
                Ok(texture) => {
                    tex.insert(
                        "minecraft:block/".to_owned()
                            + path
                                .file_stem()
                                .ok_or(format!("invalid file name: {}", path.display()))?
                                .to_str()
                                .ok_or(format!("nonunicode file name: {}", path.display()))?,
                        texture,
                    );
                }
            }
        }
    }

    info!("Loaded {} textures", tex.len());
    Ok(tex)
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

/// Add missing common blocks that are not in the generated palette
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

/// Add base colors for blocks that only have state variants
fn add_base_colors(palette: &mut HashMap<String, Rgba>) {
    info!("Adding base colors for state variants");

    let mut blocks_with_states: HashMap<String, Vec<Rgba>> = HashMap::new();
    let mut blocks_without_states = std::collections::HashSet::new();

    // Scan for state variants
    for (key, &color) in palette.iter() {
        if key.contains('|') {
            let base_name = key.split('|').next().unwrap().to_string();
            blocks_with_states
                .entry(base_name)
                .or_default()
                .push(color);
        } else {
            blocks_without_states.insert(key.clone());
        }
    }

    // Add missing base blocks
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
    info!("Assets path: {}", args.assets.display());
    info!("Output: {}", args.output.display());

    if !args.assets.exists() {
        error!("Assets path does not exist: {}", args.assets.display());
        return Err("Assets path not found".into());
    }

    // Load resources
    let textures = load_textures(&args.assets.join("textures").join("block"))?;
    let blockstates = load_blockstates(&args.assets.join("blockstates"))?;
    let models = load_models(&args.assets.join("models").join("block"))?;

    info!("Creating renderer");
    let mut renderer = Renderer::new(blockstates.clone(), models, textures.clone());
    let mut failed = 0;
    let mut mapped = 0;
    let mut success = 0;

    // Regex mappings for blocks that can't be rendered directly
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

    // 1.17 renamed grass_path to dirt_path. This hacks it back in for old region files.
    if let Some(path) = palette.get("minecraft:dirt_path").cloned() {
        palette.insert("minecraft:grass_path".into(), path);
    }

    // Add missing common blocks
    add_missing_blocks(&mut palette);

    // Add base colors for state variants (for O(1) lookup)
    add_base_colors(&mut palette);

    // Write palette.json
    info!("Writing palette to: {}", args.output.display());
    let file = std::fs::File::create(&args.output)?;
    serde_json::to_writer_pretty(file, &palette)?;

    info!(
        "✓ Palette generation complete! {} total blocks written",
        palette.len()
    );

    Ok(())
}
