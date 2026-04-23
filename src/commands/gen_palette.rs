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
//   3. texture-path probe — direct lookup at `<ns>:block/<name>` or
//      `<ns>:blocks/<name>` (pre-1.13 layout).
//   4. user overrides JSON (`--overrides`) — final authoritative precedence,
//      applied after all automatic resolution.

use clap::Args;
use fastanvil::{
    tex::{Blockstate, Model, Render, Renderer, Texture},
    Rgba,
};
use log::{debug, error, info, warn};
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
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

    /// Optional user overrides file. JSON map of `"namespace:id"` →
    /// `[r,g,b,a]`. Applied last — overrides everything automatic.
    #[arg(long)]
    overrides: Option<PathBuf>,
}

/// Averages a raw RGBA image into a single color.
/// RGB is averaged only over pixels with alpha > 0 — this prevents sparse
/// textures (vines, fences, crops, fire) from being washed toward black by
/// their transparent background. Alpha is averaged over all pixels, so
/// coverage is preserved in the output alpha channel.
/// Uses quadratic mean (RMS) for perceptually better mixing.
fn avg_colour(rgba_data: &[u8]) -> Rgba {
    let mut rgb = [0f64; 3];
    let mut alpha_sq = 0f64;
    let mut total = 0usize;
    let mut opaque = 0usize;

    for p in rgba_data.chunks(4) {
        if p.len() < 4 {
            continue;
        }
        total += 1;
        alpha_sq += (p[3] as u64 * p[3] as u64) as f64;
        if p[3] > 0 {
            rgb[0] += (p[0] as u64 * p[0] as u64) as f64;
            rgb[1] += (p[1] as u64 * p[1] as u64) as f64;
            rgb[2] += (p[2] as u64 * p[2] as u64) as f64;
            opaque += 1;
        }
    }

    if total == 0 || opaque == 0 {
        return [0, 0, 0, 0];
    }

    [
        (rgb[0] / opaque as f64).sqrt() as u8,
        (rgb[1] / opaque as f64).sqrt() as u8,
        (rgb[2] / opaque as f64).sqrt() as u8,
        (alpha_sq / total as f64).sqrt() as u8,
    ]
}

// --- Raw model/blockstate types for fallback access -------------------------
// fastanvil's `Face.texture` field is private, so we parse the same JSON into
// our own public-field structs alongside fastanvil's types. Only used when
// `Renderer::get_top` fails or for multipart blockstates (which fastanvil
// doesn't render).

#[derive(Deserialize, Debug, Clone)]
struct RawFace {
    texture: String,
}

#[derive(Deserialize, Debug, Clone)]
struct RawElement {
    #[serde(default)]
    faces: HashMap<String, RawFace>,
}

#[derive(Deserialize, Debug, Clone)]
struct RawModel {
    parent: Option<String>,
    #[serde(default)]
    textures: Option<HashMap<String, String>>,
    #[serde(default)]
    elements: Option<Vec<RawElement>>,
    /// Forge custom model loaders (e.g. `functionalstorage:framedblock`) skip
    /// standard elements and put their per-face textures inside a `children`
    /// map — one inner "sub-model" per component. We only capture enough to
    /// pull texture refs out for the last-ditch any-texture fallback.
    #[serde(default)]
    children: Option<HashMap<String, RawChild>>,
}

#[derive(Deserialize, Debug, Clone)]
struct RawChild {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    textures: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
struct RawVariantRef {
    model: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum RawVariantSpec {
    Single(RawVariantRef),
    Many(Vec<RawVariantRef>),
}

#[derive(Deserialize, Debug, Clone)]
struct RawPart {
    apply: RawVariantSpec,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
enum RawBlockstate {
    Variants(HashMap<String, RawVariantSpec>),
    Multipart(Vec<RawPart>),
}

#[derive(Default)]
struct Pools {
    blockstates: HashMap<String, Blockstate>,
    models: HashMap<String, Model>,
    textures: HashMap<String, Texture>,
    raw_blockstates: HashMap<String, RawBlockstate>,
    raw_models: HashMap<String, RawModel>,
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
                // Parse raw form for fallback access. Best-effort: a failure
                // here just means this block won't benefit from multipart /
                // side-face fallback, not that the whole pipeline breaks.
                if let Ok(raw) = serde_json::from_slice::<RawBlockstate>(&buf) {
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

// --- Raw-model helpers ------------------------------------------------------

/// Qualify an unqualified resource reference with `minecraft:`. Mirrors what
/// fastanvil does internally; the vanilla convention is that bare strings in
/// parent/texture refs default to minecraft.
fn qualify(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("minecraft:{}", name)
    }
}

/// Walk the parent chain, merging textures (child overrides parent) and
/// inheriting elements (child overrides if declared). Resolves `#ref`
/// texture variables at the end. Returns None if the root model is missing.
fn flatten_raw_model(
    name: &str,
    raw_models: &HashMap<String, RawModel>,
) -> Option<RawModel> {
    let mut chain: Vec<RawModel> = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(qualify(name));

    while let Some(key) = cur {
        if !seen.insert(key.clone()) {
            break; // cycle
        }
        let Some(m) = raw_models.get(&key) else {
            break;
        };
        chain.push(m.clone());
        cur = m.parent.as_ref().map(|p| qualify(p));
    }

    let mut out = chain.pop()?; // root-most ancestor
    // Merge descendants onto it, child-wins.
    while let Some(child) = chain.pop() {
        if let Some(ct) = child.textures {
            let pt = out.textures.get_or_insert_with(HashMap::new);
            for (k, v) in ct {
                pt.insert(k, v);
            }
        }
        if child.elements.is_some() {
            out.elements = child.elements;
        }
        if child.children.is_some() {
            out.children = child.children;
        }
    }

    if let Some(ref mut tex) = out.textures {
        resolve_texture_variables(tex);
    }
    Some(out)
}

/// Iteratively resolve `#name` references inside a texture map. Bounded to
/// a handful of passes to short-circuit any pathological input.
fn resolve_texture_variables(tex: &mut HashMap<String, String>) {
    for _ in 0..8 {
        let snapshot = tex.clone();
        let mut changed = false;
        for (_, v) in tex.iter_mut() {
            if let Some(key) = v.strip_prefix('#') {
                if let Some(target) = snapshot.get(key) {
                    if target != v {
                        *v = target.clone();
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Resolve a face's texture reference against a flattened model's texture map.
/// `#ref` → look up in the map, otherwise use as-is. Qualifies to `minecraft:`
/// if no namespace.
fn resolve_face_texture(face_tex: &str, model: &RawModel) -> Option<String> {
    let resolved = if let Some(key) = face_tex.strip_prefix('#') {
        model.textures.as_ref()?.get(key)?.clone()
    } else {
        face_tex.to_string()
    };
    Some(qualify(&resolved))
}

/// Pick the first face in a flattened model whose texture is present.
/// Preference order: up → down → side faces (block top is what matters most
/// for a top-down map; down handles blocks only visible from underneath;
/// sides are last resort).
fn render_any_face(
    model: &RawModel,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let priority = ["up", "down", "north", "south", "east", "west"];
    let elements = model.elements.as_ref()?;
    for key in &priority {
        for el in elements {
            if let Some(face) = el.faces.get(*key) {
                if let Some(tex_ref) = resolve_face_texture(&face.texture, model) {
                    if let Some(tex) = textures.get(&tex_ref) {
                        return Some(tex.clone());
                    }
                }
            }
        }
    }
    None
}

/// Given a raw variant spec, pick the first model name. Variants::Many just
/// picks element 0 (vanilla would pick by weight, but we only need color
/// and variants are visually similar).
fn first_model_name(spec: &RawVariantSpec) -> Option<&str> {
    match spec {
        RawVariantSpec::Single(v) => Some(&v.model),
        RawVariantSpec::Many(vs) => vs.first().map(|v| v.model.as_str()),
    }
}

/// Fallback for block-entity models (signs, beds, chests, banners, Botania
/// `buried_petals`/`floating_*` etc.): their `models/block/...json` has no
/// `elements` because the geometry is drawn by a tile entity renderer at
/// runtime. They do still declare a `particle` texture (the texture used for
/// break particles) that's a sensible stand-in color — oak planks for most
/// beds, magenta_wool for magenta buried petals.
fn render_particle_texture(
    model: &RawModel,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let tex_map = model.textures.as_ref()?;
    let particle = tex_map.get("particle")?;
    if particle.starts_with('#') {
        return None; // unresolved reference
    }
    textures.get(&qualify(particle)).cloned()
}

/// Last-ditch fallback for Forge custom loaders (`functionalstorage:framedblock`,
/// `minecraft:block` with only `children` etc.): scan every texture reference
/// that appears anywhere in the model — direct `textures` map, child models'
/// `textures` maps, and their flattened parent chains — and return the first
/// one whose PNG is actually in the texture pool.
fn render_any_texture_ref(
    model: &RawModel,
    raw_models: &HashMap<String, RawModel>,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    // Preferred texture-map keys first (most likely to be the main face).
    // Applied to both the root model's texture map and each child's.
    let priority_keys = ["all", "side", "top", "front", "texture", "0"];

    let scan_map = |map: &HashMap<String, String>| -> Option<Texture> {
        for k in &priority_keys {
            if let Some(v) = map.get(*k) {
                if !v.starts_with('#') {
                    if let Some(tex) = textures.get(&qualify(v)) {
                        return Some(tex.clone());
                    }
                }
            }
        }
        for (k, v) in map {
            if k == "particle" || v.starts_with('#') {
                continue; // particle handled by its own tier; skip refs
            }
            if let Some(tex) = textures.get(&qualify(v)) {
                return Some(tex.clone());
            }
        }
        None
    };

    if let Some(map) = &model.textures {
        if let Some(tex) = scan_map(map) {
            return Some(tex);
        }
    }
    let children = model.children.as_ref()?;
    for child in children.values() {
        if let Some(parent) = &child.parent {
            if let Some(flat) = flatten_raw_model(parent, raw_models) {
                if let Some(map) = &flat.textures {
                    if let Some(tex) = scan_map(map) {
                        return Some(tex);
                    }
                }
            }
        }
        if let Some(map) = &child.textures {
            if let Some(tex) = scan_map(map) {
                return Some(tex);
            }
        }
    }
    None
}

/// Walk variants (preferring `upper`/`top` keys for tall/double blocks) or
/// multipart parts in a blockstate, flatten each referenced model, and hand
/// the model to `choose`. First strategy-returned texture wins.
fn render_any_variant_of_block(
    raw_bs: &RawBlockstate,
    raw_models: &HashMap<String, RawModel>,
    mut choose: impl FnMut(&RawModel) -> Option<Texture>,
) -> Option<Texture> {
    match raw_bs {
        RawBlockstate::Variants(vars) => {
            // Heuristic: tall plants / double slabs only render from one half.
            // Prefer keys containing "upper" or "top" — matches mcasaenk.
            let mut keys: Vec<&String> = vars.keys().collect();
            keys.sort_by_key(|k| {
                if k.contains("upper") || k.contains("top") {
                    0
                } else if k.is_empty() {
                    1
                } else {
                    2
                }
            });
            for key in keys {
                let Some(model_name) = first_model_name(&vars[key]) else {
                    continue;
                };
                if let Some(model) = flatten_raw_model(model_name, raw_models) {
                    if let Some(tex) = choose(&model) {
                        return Some(tex);
                    }
                }
            }
            None
        }
        RawBlockstate::Multipart(parts) => {
            for part in parts {
                let Some(model_name) = first_model_name(&part.apply) else {
                    continue;
                };
                if let Some(model) = flatten_raw_model(model_name, raw_models) {
                    if let Some(tex) = choose(&model) {
                        return Some(tex);
                    }
                }
            }
            None
        }
    }
}

/// Last-resort fallback: look for a texture whose path mirrors the block ID.
/// `mymod:steel_block` → try `mymod:block/steel_block`, then pre-1.13
/// `mymod:blocks/steel_block`. Useful for mods whose blockstate/model JSONs
/// are broken or unconventional but whose textures follow the standard layout.
fn probe_texture_by_name(
    block_name: &str,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let (ns, name) = block_name.split_once(':')?;
    for prefix in ["block", "blocks"] {
        let candidate = format!("{}:{}/{}", ns, prefix, name);
        if let Some(tex) = textures.get(&candidate) {
            return Some(tex.clone());
        }
    }
    None
}

/// Per-tier success counters. Used only for the final resolution summary.
#[derive(Default)]
struct Counters {
    rendered: usize,
    side_fallback: usize,
    particle: usize,
    any_texture: usize,
    mapped: usize,
    probed: usize,
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
/// (water, lava, air, etc.).
fn add_missing_blocks(palette: &mut HashMap<String, Rgba>) {
    info!("Adding missing common blocks");

    let missing = vec![
        ("minecraft:air", [0, 0, 0, 0]),
        ("minecraft:cave_air", [0, 0, 0, 0]),
        ("minecraft:void_air", [0, 0, 0, 0]),
        ("minecraft:water", [63, 118, 228, 180]),
        ("minecraft:flowing_water", [63, 118, 228, 180]),
        ("minecraft:bubble_column", [63, 118, 228, 180]),
        ("minecraft:lava", [207, 78, 0, 255]),
        ("minecraft:flowing_lava", [207, 78, 0, 255]),
        ("minecraft:vine", [106, 136, 44, 200]),
        ("minecraft:grass", [124, 189, 107, 255]),
        ("minecraft:fern", [104, 149, 92, 255]),
        // Technical / admin blocks that are invisible in-world but still
        // appear as palette entries the chunk may query.
        ("minecraft:barrier", [0, 0, 0, 0]),
        ("minecraft:moving_piston", [0, 0, 0, 0]),
        ("minecraft:light", [0, 0, 0, 0]),
        ("minecraft:structure_void", [0, 0, 0, 0]),
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

/// Parse a user overrides file. Format: `{"namespace:id": [r,g,b,a], ...}`.
fn load_overrides(path: &Path) -> Result<HashMap<String, Rgba>> {
    let file = File::open(path)?;
    let map: HashMap<String, [u8; 4]> = serde_json::from_reader(file)?;
    Ok(map)
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
