use fastanvil::tex::{Blockstate, Model, Texture};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use super::Result;
use super::raw::{RawBlockstate, RawModel, parse_blockstate_lenient};

#[derive(Default)]
pub(crate) struct Pools {
    pub(crate) blockstates: HashMap<String, Blockstate>,
    pub(crate) models: HashMap<String, Model>,
    pub(crate) textures: HashMap<String, Texture>,
    pub(crate) raw_blockstates: HashMap<String, RawBlockstate>,
    pub(crate) raw_models: HashMap<String, RawModel>,
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
///
/// Forge packs the extra mods they depend on as Jar-in-Jar (JIJ) entries under
/// `META-INF/jarjar/*.jar`. Forge extracts those at runtime, so from the game's
/// point of view the nested mod's assets are fully available. We do the same:
/// after reading the outer archive's own assets, recurse into each nested jar
/// with the same `pools` (so outer assets still win on conflict). Nested jars
/// may themselves contain JIJ entries — recursion handles any depth.
fn load_archive_from_reader<R: Read + Seek>(
    label: &str,
    reader: R,
    pools: &mut Pools,
) -> Result<()> {
    let mut archive = ZipArchive::new(reader)?;

    let mut bs_added = 0usize;
    let mut m_added = 0usize;
    let mut t_added = 0usize;

    // Defer nested jars so the outer archive's own assets are registered first
    // (outer wins on conflict).
    let mut nested_jars: Vec<(String, Vec<u8>)> = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();

        if is_jarjar_entry(&name) {
            let mut buf = Vec::new();
            if let Err(e) = entry.read_to_end(&mut buf) {
                debug!("Failed to read nested jar {}: {}", name, e);
                continue;
            }
            nested_jars.push((name, buf));
            continue;
        }

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
                        pools.blockstates.insert(key.clone(), bs);
                        bs_added += 1;
                    }
                    Err(e) => debug!("Failed to parse blockstate {}: {}", name, e),
                }
                // Parse raw form for fallback access. Lenient: handles both
                // vanilla `{variants}/{multipart}` and Forge 1.12.2's
                // `{forge_marker: 1, defaults, variants}` shape — most 1.12.2
                // mods ship the latter, and falling through here is what makes
                // those blocks resolvable beyond bare-id gray.
                if let Some(raw) = parse_blockstate_lenient(&buf) {
                    pools.raw_blockstates.insert(key, raw);
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
                        pools.models.insert(key.clone(), m);
                        m_added += 1;
                    }
                    Err(e) => debug!("Failed to parse model {}: {}", name, e),
                }
                if let Ok(raw) = serde_json::from_slice::<RawModel>(&buf) {
                    pools.raw_models.insert(key, raw);
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
        "  [{}] + {} blockstates, {} models, {} textures",
        label, bs_added, m_added, t_added
    );

    for (entry_name, buf) in nested_jars {
        let short = entry_name
            .rsplit('/')
            .next()
            .unwrap_or(entry_name.as_str());
        let nested_label = format!("{} > {}", label, short);
        if let Err(e) = load_archive_from_reader(&nested_label, Cursor::new(buf), pools) {
            warn!("Failed to load nested {}: {}", entry_name, e);
        }
    }

    Ok(())
}

/// True for JIJ entries — nested mod jars Forge (and friends) bundle under
/// `META-INF/jarjar/`. Filters out the `metadata.json` sibling and any
/// non-jar files sharing that directory.
fn is_jarjar_entry(name: &str) -> bool {
    name.starts_with("META-INF/jarjar/")
        && name
            .rsplit('.')
            .next()
            .map(|e| e.eq_ignore_ascii_case("jar"))
            .unwrap_or(false)
}

fn load_archive(path: &Path, pools: &mut Pools) -> Result<()> {
    let file = File::open(path)?;
    let label = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("archive")
        .to_string();
    load_archive_from_reader(&label, file, pools)
}

pub(crate) fn load_packs(paths: &[PathBuf]) -> Result<Pools> {
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
