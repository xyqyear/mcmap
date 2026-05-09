use fastanvil::tex::{Blockstate, Model, Texture};
use log::{debug, error, info, warn};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use super::raw::{RawBlockstate, RawModel, parse_blockstate_lenient, scrub_extension_keys};
use crate::commands::gen_palette::shared::progress::PackLoadReport;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
pub struct Pools {
    pub blockstates: HashMap<String, Blockstate>,
    pub models: HashMap<String, Model>,
    pub textures: HashMap<String, Texture>,
    pub raw_blockstates: HashMap<String, RawBlockstate>,
    pub raw_models: HashMap<String, RawModel>,
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
/// Forge packs extra mod dependencies as Jar-in-Jar (JIJ) entries under
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
                // Mod-extension siblings of `variants` / `multipart` (see
                // `scrub_extension_keys`) break externally-tagged enum
                // deserialization for both the strict and lenient parsers.
                // Scrub them once; both parsers see the cleaned bytes.
                let parse_buf: std::borrow::Cow<[u8]> = match scrub_extension_keys(&buf) {
                    Some(cleaned) => std::borrow::Cow::Owned(cleaned),
                    None => std::borrow::Cow::Borrowed(buf.as_slice()),
                };
                match serde_json::from_slice::<Blockstate>(&parse_buf) {
                    Ok(bs) => {
                        pools.blockstates.insert(key.clone(), bs);
                        bs_added += 1;
                    }
                    Err(e) => debug!("Failed to parse blockstate {}: {}", name, e),
                }
                // Lenient raw parse for fallback access — handles both vanilla
                // `{variants}/{multipart}` and Forge 1.12.2's
                // `{forge_marker: 1, defaults, variants}` shape. Most 1.12.2
                // mods ship the latter; falling through here is what makes
                // those blocks resolvable beyond bare-id gray.
                if let Some(raw) = parse_blockstate_lenient(&parse_buf) {
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
        let short = entry_name.rsplit('/').next().unwrap_or(entry_name.as_str());
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

pub fn load_packs<F: FnMut(&PackLoadReport)>(
    paths: &[PathBuf],
    mut on_pack: F,
) -> Result<Pools> {
    let archives = expand_packs(paths)?;
    if archives.is_empty() {
        return Err("No pack files to load (did you pass empty directories?)".into());
    }

    let mut pools = Pools::default();
    let total = archives.len();
    for (i, archive_path) in archives.iter().enumerate() {
        info!("Loading pack: {}", archive_path.display());
        let before = (
            pools.blockstates.len(),
            pools.models.len(),
            pools.textures.len(),
        );
        let err = match load_archive(archive_path, &mut pools) {
            Ok(()) => None,
            Err(e) => {
                error!("Failed to load {}: {}", archive_path.display(), e);
                Some(e.to_string())
            }
        };
        let after = (
            pools.blockstates.len(),
            pools.models.len(),
            pools.textures.len(),
        );
        on_pack(&PackLoadReport {
            path: archive_path.clone(),
            index: i + 1,
            total,
            blockstates_added: after.0 - before.0,
            models_added: after.1 - before.1,
            textures_added: after.2 - before.2,
            error: err,
        });
    }

    info!(
        "Totals: {} blockstates, {} models, {} textures across {} pack(s)",
        pools.blockstates.len(),
        pools.models.len(),
        pools.textures.len(),
        archives.len()
    );

    break_model_parent_cycles(&mut pools.models);

    Ok(pools)
}

/// Walk each model's `parent` chain, snipping the edge that closes a cycle.
///
/// fastanvil 0.32's `Renderer::flatten_model` walks `parent` without cycle
/// detection, so any model whose parent chain loops back on itself causes
/// `Renderer::get_top` to spin forever. Real-world example:
/// `vscontrolcraft:block/propeller_controller/block` ships with
/// `"parent": "vscontrolcraft:block/propeller_controller/block"` — a self-
/// reference. After this pass, fastanvil sees a cycle-free graph; the offending
/// model still resolves via its own `elements` (which the snipped parent edge
/// would have inherited identically anyway).
///
/// Resolution mirrors fastanvil's `get_model`: try the parent string as-is,
/// then with a `minecraft:` prefix. Missing parents are left alone — fastanvil
/// surfaces them as `MissingModel` at render time.
fn break_model_parent_cycles(models: &mut HashMap<String, Model>) {
    let keys: Vec<String> = models.keys().cloned().collect();
    let mut snipped = 0usize;
    for start in keys {
        let mut current = start;
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            if !seen.insert(current.clone()) {
                break;
            }
            let parent_raw = match models.get(&current).and_then(|m| m.parent.as_ref()) {
                Some(p) => p.clone(),
                None => break,
            };
            let parent_key = if models.contains_key(&parent_raw) {
                parent_raw.clone()
            } else {
                let mc = format!("minecraft:{}", parent_raw);
                if models.contains_key(&mc) {
                    mc
                } else {
                    break;
                }
            };
            if seen.contains(&parent_key) {
                if let Some(m) = models.get_mut(&current) {
                    m.parent = None;
                }
                warn!(
                    "Snipped cyclic parent on model {} (chain re-entered {})",
                    current, parent_key
                );
                snipped += 1;
                break;
            }
            current = parent_key;
        }
    }
    if snipped > 0 {
        info!("Broke {} cyclic model parent chain(s)", snipped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastanvil::tex::Model;

    fn empty_model(parent: Option<&str>) -> Model {
        Model {
            parent: parent.map(|s| s.to_string()),
            textures: None,
            elements: None,
        }
    }

    #[test]
    fn snips_self_cycle() {
        let mut models = HashMap::new();
        models.insert(
            "ns:block/self".to_string(),
            empty_model(Some("ns:block/self")),
        );
        break_model_parent_cycles(&mut models);
        assert_eq!(models["ns:block/self"].parent, None);
    }

    #[test]
    fn snips_two_step_cycle() {
        let mut models = HashMap::new();
        models.insert("ns:block/a".to_string(), empty_model(Some("ns:block/b")));
        models.insert("ns:block/b".to_string(), empty_model(Some("ns:block/a")));
        break_model_parent_cycles(&mut models);
        // Whichever node was visited second gets its parent snipped — exactly
        // one of the two edges should be broken so the chain terminates.
        let a_parent = models["ns:block/a"].parent.as_deref();
        let b_parent = models["ns:block/b"].parent.as_deref();
        assert!(
            (a_parent.is_none() && b_parent == Some("ns:block/a"))
                || (b_parent.is_none() && a_parent == Some("ns:block/b")),
            "expected one edge snipped, got a={:?} b={:?}",
            a_parent,
            b_parent
        );
    }

    #[test]
    fn leaves_clean_chain_alone() {
        let mut models = HashMap::new();
        models.insert("ns:block/leaf".to_string(), empty_model(Some("ns:block/mid")));
        models.insert("ns:block/mid".to_string(), empty_model(Some("ns:block/root")));
        models.insert("ns:block/root".to_string(), empty_model(None));
        break_model_parent_cycles(&mut models);
        assert_eq!(models["ns:block/leaf"].parent.as_deref(), Some("ns:block/mid"));
        assert_eq!(models["ns:block/mid"].parent.as_deref(), Some("ns:block/root"));
        assert_eq!(models["ns:block/root"].parent, None);
    }

    #[test]
    fn resolves_minecraft_prefix_fallback() {
        // Vanilla parents are often written as bare `block/cube_all`; fastanvil's
        // `get_model` prepends `minecraft:` when the bare key misses. The cycle
        // walker has to mirror that or it misses cycles that span the gap.
        let mut models = HashMap::new();
        models.insert(
            "minecraft:block/cube_all".to_string(),
            empty_model(Some("block/cube_all")),
        );
        break_model_parent_cycles(&mut models);
        assert_eq!(models["minecraft:block/cube_all"].parent, None);
    }
}
