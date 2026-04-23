// Texture-only pack loader for 1.7.10.
//
// The modern gen-palette path loads blockstates, models, and textures because
// it renders block faces through `fastanvil::tex::Renderer`. 1.7.10 packs
// don't ship blockstate/model JSONs — just raw PNGs under
// `assets/<ns>/textures/blocks/` — so we only index those. Cheaper, simpler,
// and avoids pulling unrelated JSON parse errors into this path.

use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use super::Result;

/// One texture pack, indexed by `namespace:relative/path/without_ext`.
/// The key mirrors the modern pack format so vanilla-style keys like
/// `"minecraft:block/stone"` (by renaming `textures/blocks/` → `block/`) stay
/// consistent.
pub struct TexturePack {
    #[allow(dead_code)] // useful for debug logging if we surface it later
    pub label: String,
    /// Key format: `"<namespace>:block/<name>"` (matches the modern
    /// resource-pack path convention, with `textures/blocks/` collapsed to
    /// just `block/`). Vanilla 1.7.10 uses `textures/blocks/` but this key
    /// normalization lets the resolver use one-shot lookups regardless of
    /// which convention the pack follows.
    ///
    /// Value is raw RGBA bytes (same shape the modern path produces).
    pub textures: HashMap<String, Vec<u8>>,
    /// Case-insensitive map from `"<ns_lc>:block/<name_lc>"` to the original
    /// pack key, used to recover from case-mismatched namespaces like
    /// `HardcoreEnderExpansion:ender_goo` vs `assets/hardcoreenderexpansion/`.
    pub textures_ci: HashMap<String, String>,
}

impl TexturePack {
    fn new(label: String) -> Self {
        Self {
            label,
            textures: HashMap::new(),
            textures_ci: HashMap::new(),
        }
    }

    fn insert(&mut self, key: String, data: Vec<u8>) {
        if self.textures.contains_key(&key) {
            return; // first-wins within a pack
        }
        let lc = key.to_lowercase();
        self.textures_ci.insert(lc, key.clone());
        self.textures.insert(key, data);
    }
}

/// Expand user-supplied paths (files or directories) into an ordered list of
/// archive paths. Mirrors the modern loader's logic.
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

/// Parse a zip entry path into (namespace, name-without-ext). Accepts any of:
///   - `assets/<ns>/textures/blocks/<rest>.png`   (1.7.10 convention)
///   - `assets/<ns>/textures/block/<rest>.png`    (1.13+ convention — seen in
///                                                some rebranded packs)
/// Returns `(ns, "<rest>")` — the caller forms the final key.
fn parse_texture_entry(entry_name: &str) -> Option<(String, String)> {
    let s = entry_name.trim_start_matches('/').trim_start_matches("./");
    // Split into up to 5 parts: assets / <ns> / textures / <kind> / <rest>
    let mut parts = s.splitn(5, '/');
    let assets = parts.next()?;
    if assets != "assets" {
        return None;
    }
    let ns = parts.next()?;
    let textures = parts.next()?;
    if textures != "textures" {
        return None;
    }
    let kind = parts.next()?;
    let rest = parts.next()?;
    if ns.is_empty() || rest.is_empty() {
        return None;
    }
    // Only block textures — this is a top-down renderer.
    if kind != "blocks" && kind != "block" {
        return None;
    }
    let (rest_no_ext, ext) = rest.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("png") {
        return None;
    }
    Some((ns.to_string(), rest_no_ext.to_string()))
}

/// Load every archive, first-wins on texture key. Recurses into
/// `META-INF/jarjar/*.jar` the same way the modern loader does — Forge
/// packages extra mods as JIJ, and those can contain further blocks.
fn load_archive_from_reader<R: Read + Seek>(
    label: &str,
    reader: R,
    pack: &mut TexturePack,
) -> Result<()> {
    let mut archive = ZipArchive::new(reader)?;
    let mut added = 0usize;
    let mut nested: Vec<(String, Vec<u8>)> = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if is_jarjar_entry(&name) {
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_err() {
                continue;
            }
            nested.push((name, buf));
            continue;
        }
        let Some((ns, rest_no_ext)) = parse_texture_entry(&name) else {
            continue;
        };
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        match image::load_from_memory(&buf) {
            Ok(img) => {
                // Some textures are animation filmstrips (width < height, N
                // vertical frames). Crop to a square from the top to keep
                // single-frame averaging semantics.
                let img = normalize_animation_strip(img);
                let rgba = img.to_rgba8().into_raw();
                let key = format!("{}:block/{}", ns, rest_no_ext);
                if !pack.textures.contains_key(&key) {
                    pack.insert(key, rgba);
                    added += 1;
                }
            }
            Err(e) => {
                debug!("Failed to decode texture {}: {}", name, e);
            }
        }
    }

    info!("  [{}] + {} textures", label, added);

    for (entry_name, buf) in nested {
        let short = entry_name
            .rsplit('/')
            .next()
            .unwrap_or(entry_name.as_str());
        let nested_label = format!("{} > {}", label, short);
        if let Err(e) = load_archive_from_reader(&nested_label, Cursor::new(buf), pack) {
            warn!("Failed to load nested {}: {}", entry_name, e);
        }
    }
    Ok(())
}

fn is_jarjar_entry(name: &str) -> bool {
    name.starts_with("META-INF/jarjar/")
        && name
            .rsplit('.')
            .next()
            .map(|e| e.eq_ignore_ascii_case("jar"))
            .unwrap_or(false)
}

/// If an image is much taller than wide (typical animation strips), crop it
/// to a top square so the averaged color reflects a single frame instead of
/// the whole reel. 1.7.10 worlds rely on this heavily (water, lava, fire).
fn normalize_animation_strip(img: image::DynamicImage) -> image::DynamicImage {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if h > w * 2 {
        img.crop_imm(0, 0, w, w)
    } else {
        img
    }
}

fn load_archive(path: &Path, pack: &mut TexturePack) -> Result<()> {
    let file = File::open(path)?;
    let label = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("archive")
        .to_string();
    load_archive_from_reader(&label, file, pack)
}

/// Load every pack into its own `TexturePack`. Pack order is preserved for
/// first-wins semantics during resolution (see `resolve_modded`).
pub fn load_texture_packs(paths: &[PathBuf]) -> Result<Vec<TexturePack>> {
    let archives = expand_packs(paths)?;
    if archives.is_empty() {
        return Err("No pack files to load (did you pass empty directories?)".into());
    }
    let mut packs = Vec::with_capacity(archives.len());
    for archive_path in &archives {
        let label = archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("archive")
            .to_string();
        info!("Loading pack: {}", archive_path.display());
        let mut pack = TexturePack::new(label.clone());
        if let Err(e) = load_archive(archive_path, &mut pack) {
            warn!("Failed to load {}: {}", archive_path.display(), e);
        }
        packs.push(pack);
    }
    Ok(packs)
}
