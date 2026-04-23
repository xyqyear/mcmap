use fastanvil::Rgba;
use fastanvil::tex::{Render, Renderer, Texture};
use log::debug;
use regex::Regex;
use std::collections::HashMap;

use super::color::avg_colour;
use super::raw::{
    RawBlockstate, RawModel, RawVariantRef, RawVariantSpec, flatten_raw_model,
    flatten_raw_model_with_overrides, qualify, resolve_face_texture,
};

/// Pick the first face in a flattened model whose texture is present.
/// Preference order: up → down → side faces (block top is what matters most
/// for a top-down map; down handles blocks only visible from underneath;
/// sides are last resort).
pub(crate) fn render_any_face(
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

/// Fallback for block-entity models (signs, beds, chests, banners, Botania
/// `buried_petals`/`floating_*` etc.): their `models/block/...json` has no
/// `elements` because the geometry is drawn by a tile entity renderer at
/// runtime. They do still declare a `particle` texture (the texture used for
/// break particles) that's a sensible stand-in color — oak planks for most
/// beds, magenta_wool for magenta buried petals.
pub(crate) fn render_particle_texture(
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
pub(crate) fn render_any_texture_ref(
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
/// the model to `choose`. First strategy-returned texture wins. Forge
/// blockstates carry per-variant `textures` overrides on the variant ref —
/// these are merged onto the flattened model before `choose` sees it.
pub(crate) fn render_any_variant_of_block(
    raw_bs: &RawBlockstate,
    raw_models: &HashMap<String, RawModel>,
    mut choose: impl FnMut(&RawModel) -> Option<Texture>,
) -> Option<Texture> {
    let try_ref = |r: &RawVariantRef, choose: &mut dyn FnMut(&RawModel) -> Option<Texture>| {
        let model = flatten_raw_model_with_overrides(&r.model, raw_models, r.textures.as_ref())?;
        choose(&model)
    };
    let try_spec = |spec: &RawVariantSpec,
                    choose: &mut dyn FnMut(&RawModel) -> Option<Texture>|
     -> Option<Texture> {
        match spec {
            RawVariantSpec::Single(r) => try_ref(r, choose),
            RawVariantSpec::Many(rs) => {
                for r in rs {
                    if let Some(t) = try_ref(r, choose) {
                        return Some(t);
                    }
                }
                None
            }
        }
    };

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
                if let Some(t) = try_spec(&vars[key], &mut choose) {
                    return Some(t);
                }
            }
            None
        }
        RawBlockstate::Multipart(parts) => {
            for part in parts {
                if let Some(t) = try_spec(&part.apply, &mut choose) {
                    return Some(t);
                }
            }
            None
        }
    }
}

/// Last-resort fallback: probe a handful of texture-path patterns derived
/// from the block name. Modern (1.13+) packs use `<ns>:block/<name>`;
/// 1.12.2-vintage packs use `<ns>:blocks/<name>` (plural). Many mods bury
/// per-variant textures one directory deeper (e.g. `nuclearcraft:blocks/
/// ingot_block/copper`); for those we walk the texture pool and accept the
/// first key under `<ns>:blocks/<name>/...` or `<ns>:block/<name>/...`.
/// Several mods also ship a generic-stem variant (`<name>` minus a leading
/// `block_` / `block` / trailing `_block`); try those too.
pub(crate) fn probe_texture_by_name(
    block_name: &str,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let (ns, name) = block_name.split_once(':')?;
    let stems = derive_block_stems(name);

    for stem in &stems {
        for prefix in ["block", "blocks"] {
            let key = format!("{}:{}/{}", ns, prefix, stem);
            if let Some(tex) = textures.get(&key) {
                return Some(tex.clone());
            }
        }
    }

    // Per-variant directory: pick the first texture under `<ns>:<prefix>/<stem>/`
    // — many mods key variants as `nuclearcraft:blocks/ingot_block/copper`.
    // Sort for determinism (HashMap iter order is unstable across runs).
    for stem in &stems {
        for prefix in ["block", "blocks"] {
            let dir = format!("{}:{}/{}/", ns, prefix, stem);
            let mut matches: Vec<&String> =
                textures.keys().filter(|k| k.starts_with(&dir)).collect();
            matches.sort();
            if let Some(key) = matches.first() {
                if let Some(tex) = textures.get(*key) {
                    return Some(tex.clone());
                }
            }
        }
    }

    None
}

/// Derive a few common "stem" candidates from a block local name. Drives
/// both the direct probe (`probe_texture_by_name`) and the substring fallback
/// (`probe_texture_by_substring`). Order matters — earliest entries are
/// tried first.
fn derive_block_stems(name: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![name.to_string()];
    let trimmed_digits = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed_digits != name && !trimmed_digits.is_empty() {
        out.push(trimmed_digits.to_string());
    }
    if let Some(s) = name.strip_prefix("block_") {
        out.push(s.to_string());
    }
    if let Some(s) = name.strip_prefix("block") {
        // "block" alone, or names like "blocks_foo", aren't real strips.
        if !s.is_empty() && !s.starts_with('s') && !s.starts_with('_') {
            out.push(s.to_string());
        }
    }
    if let Some(s) = name.strip_suffix("_block") {
        out.push(s.to_string());
    }
    out
}

/// Substring-match fallback: many mods (chisel, contenttweaker etc.) use
/// custom state mappers so the registered block name doesn't line up with
/// any blockstate or texture file path directly. As a last resort, walk all
/// textures in the same namespace and accept the first whose path contains
/// the block's stem as a *whole component* (between `/`/`-`/`_`/`:`/`.`
/// boundaries — substring inside a longer word doesn't count). Stems
/// shorter than 4 chars are ignored to avoid spurious matches like "ice"
/// catching anything containing "iced".
pub(crate) fn probe_texture_by_substring(
    block_name: &str,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let (ns, name) = block_name.split_once(':')?;
    let stems = derive_block_stems(name);
    let ns_prefix = format!("{}:", ns);

    let mut candidates: Vec<&String> = textures
        .keys()
        .filter(|k| k.starts_with(&ns_prefix))
        .collect();
    candidates.sort();

    for stem in &stems {
        if stem.len() < 4 {
            continue;
        }
        for k in &candidates {
            if texture_path_contains_segment(k, stem) {
                if let Some(tex) = textures.get(*k) {
                    return Some(tex.clone());
                }
            }
        }
    }
    None
}

fn texture_path_contains_segment(tex_key: &str, segment: &str) -> bool {
    tex_key
        .split(|c: char| matches!(c, '/' | '-' | '_' | ':' | '.'))
        .any(|seg| seg == segment)
}

/// Per-tier success counters. Used only for the final resolution summary.
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) rendered: usize,
    pub(crate) side_fallback: usize,
    pub(crate) particle: usize,
    pub(crate) any_texture: usize,
    pub(crate) mapped: usize,
    pub(crate) probed: usize,
    pub(crate) substring: usize,
    pub(crate) generic_blockstate: usize,
}

/// Rank sibling blockstates in the same namespace whose underscore-stripped
/// local name forms either a prefix or suffix of the queried block's local
/// name. Bridges dynamically-registered blocks to the generic family
/// blockstate they share:
/// - prefix: `chisel:carpet_red` → `chisel:carpet`, `chisel:basalt2` →
///   `chisel:basalt`, `chisel:glasspane` → `chisel:glass` (variant suffix
///   tacked on the registered name; the prefix tends to be the meaningful
///   material family).
/// - suffix: `modularmachinery:zero_factor_converter_factory_controller` →
///   `modularmachinery:blockfactorycontroller` (registered names build up
///   ahead of a generic family stem).
///
/// Returns candidates sorted by stem length descending (most specific
/// first), so the caller can fall through to a less-specific generic if
/// the most specific one's textures don't resolve.
///
/// Match rules:
/// - Both names normalized: lowercased, all non-alphanumerics dropped.
/// - The candidate is also tried with a leading `block` stripped (so
///   `blockfactorycontroller` matches a name ending in `factory_controller`).
/// - Suffix matches require ≥6 chars (controllers/etc. tend to be long
///   compounds; short suffix matches like `_iron` are noisy). Prefix
///   matches allow ≥4 chars since material families are short and the
///   stem ordering already pins the match.
pub(crate) fn find_generic_blockstates<'a>(
    block_name: &str,
    raw_blockstates: &'a HashMap<String, RawBlockstate>,
) -> Vec<&'a RawBlockstate> {
    const MIN_PREFIX: usize = 4;
    const MIN_SUFFIX: usize = 6;

    let Some((ns, name)) = block_name.split_once(':') else {
        return Vec::new();
    };
    let name_norm = normalize_for_match(name);
    if name_norm.len() < MIN_PREFIX {
        return Vec::new();
    }
    let prefix = format!("{}:", ns);

    let mut hits: Vec<(usize, &str)> = Vec::new();
    for key in raw_blockstates.keys() {
        if !key.starts_with(&prefix) {
            continue;
        }
        let bs_local = &key[prefix.len()..];
        let bs_norm = normalize_for_match(bs_local);
        if bs_norm == name_norm {
            // Direct match would have been found by the primary lookup.
            continue;
        }
        let candidates = [bs_norm.as_str(), bs_norm.strip_prefix("block").unwrap_or("")];
        let mut best_for_key: Option<usize> = None;
        for cand in candidates {
            if cand.is_empty() || cand == name_norm {
                continue;
            }
            let suffix_ok = cand.len() >= MIN_SUFFIX && name_norm.ends_with(cand);
            let prefix_ok = cand.len() >= MIN_PREFIX && name_norm.starts_with(cand);
            if suffix_ok || prefix_ok {
                let len = cand.len();
                best_for_key = Some(best_for_key.map_or(len, |b| b.max(len)));
            }
        }
        if let Some(len) = best_for_key {
            hits.push((len, key.as_str()));
        }
    }
    // Longest-stem-first; ties broken alphabetically for determinism.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    hits.into_iter()
        .filter_map(|(_, k)| raw_blockstates.get(k))
        .collect()
}

fn normalize_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[derive(Debug)]
pub(crate) struct RegexMapping {
    pub(crate) blockstate: Regex,
    pub(crate) texture_template: &'static str,
}

impl RegexMapping {
    pub(crate) fn apply(&self, blockstate: &str) -> Option<String> {
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

/// Tiered blockstate → color resolver shared by both the modern (1.13+) and
/// Forge-1.12.2 (REI) palette commands. Walks blockstate JSON via fastanvil's
/// renderer first; on failure falls back through raw model heuristics, regex
/// rewrites for naming patterns, and finally a direct texture-path probe.
pub(crate) struct Resolver<'a> {
    pub(crate) renderer: &'a mut Renderer,
    pub(crate) raw_blockstates: &'a HashMap<String, RawBlockstate>,
    pub(crate) raw_models: &'a HashMap<String, RawModel>,
    pub(crate) textures: &'a HashMap<String, Texture>,
    pub(crate) mappings: &'a [RegexMapping],
    pub(crate) counters: &'a mut Counters,
}

impl<'a> Resolver<'a> {
    /// Try every resolution tier in order. Returns `None` only when nothing
    /// succeeds — the caller decides whether to substitute a placeholder.
    pub(crate) fn resolve(&mut self, name: &str, props: Option<&str>) -> Option<Rgba> {
        // Tier 0: fastanvil renderer on the exact variant (only when caller
        // supplied properties — avoids a guaranteed-fail call for multipart
        // / single-variant blocks).
        if let Some(p) = props {
            if let Ok(tex) = self.renderer.get_top(name, p) {
                self.counters.rendered += 1;
                return Some(avg_colour(&tex));
            }
        }
        if let Some(raw_bs) = self.raw_blockstates.get(name) {
            if let Some(rgba) = self.resolve_blockstate(raw_bs) {
                return Some(rgba);
            }
        }
        // Tier 2: regex rewrites (generic + vanilla quirks).
        for mapping in self.mappings {
            if let Some(tex_name) = mapping.apply(name) {
                if let Some(tex) = self.textures.get(&tex_name) {
                    debug!("Regex mapped {} → {}", name, tex_name);
                    self.counters.mapped += 1;
                    return Some(avg_colour(tex));
                }
            }
        }
        // Tier 3: direct texture-path probe by block name.
        if let Some(tex) = probe_texture_by_name(name, self.textures) {
            debug!("Probed texture for {}", name);
            self.counters.probed += 1;
            return Some(avg_colour(&tex));
        }
        // Tier 4: substring-match texture probe across the namespace —
        // catches custom state mappers (chisel `blockaluminum` → `metals/
        // aluminum/...`) and dynamically-named blocks whose textures are
        // grouped by material rather than block id.
        if let Some(tex) = probe_texture_by_substring(name, self.textures) {
            debug!("Substring-probed texture for {}", name);
            self.counters.substring += 1;
            return Some(avg_colour(&tex));
        }
        // Tier 5: bridge dynamically-named blocks to a sibling generic
        // blockstate (modularmachinery's per-machine controllers all share
        // `blockfactorycontroller` / `blockcontroller`). Walks candidates
        // longest-stem-first and uses the first one whose textures
        // actually resolve — necessary because the closest stem isn't
        // always the one with usable assets (chisel `glasspane*` variants
        // bridge to `glass` if `glasspane`'s referenced textures are
        // missing).
        let candidates = find_generic_blockstates(name, self.raw_blockstates);
        if !candidates.is_empty() {
            // Snapshot tier counters; if the generic resolves, attribute the
            // hit to the generic_blockstate counter rather than letting it
            // inflate side/particle/any_texture.
            let snap = (
                self.counters.side_fallback,
                self.counters.particle,
                self.counters.any_texture,
            );
            for generic in candidates {
                if let Some(rgba) = self.resolve_blockstate(generic) {
                    self.counters.side_fallback = snap.0;
                    self.counters.particle = snap.1;
                    self.counters.any_texture = snap.2;
                    self.counters.generic_blockstate += 1;
                    debug!("Generic-blockstate resolved {}", name);
                    return Some(rgba);
                }
            }
            self.counters.side_fallback = snap.0;
            self.counters.particle = snap.1;
            self.counters.any_texture = snap.2;
        }
        None
    }

    /// Run the three blockstate-driven tiers against a given blockstate.
    /// Factored out so the same pipeline can be re-applied to a fallback
    /// generic blockstate without duplicating the chain.
    fn resolve_blockstate(&mut self, raw_bs: &RawBlockstate) -> Option<Rgba> {
        // Tier 1: any-face across variants / multipart parts.
        if let Some(tex) = render_any_variant_of_block(raw_bs, self.raw_models, |m| {
            render_any_face(m, self.textures)
        }) {
            self.counters.side_fallback += 1;
            return Some(avg_colour(&tex));
        }
        // Tier 1.5: particle-texture fallback for tile-entity-rendered
        // blocks (signs, beds, chests, ...).
        if let Some(tex) = render_any_variant_of_block(raw_bs, self.raw_models, |m| {
            render_particle_texture(m, self.textures)
        }) {
            self.counters.particle += 1;
            return Some(avg_colour(&tex));
        }
        // Tier 1.7: any texture reference anywhere in the model tree —
        // catches Forge custom loaders that bypass `elements`.
        if let Some(tex) = render_any_variant_of_block(raw_bs, self.raw_models, |m| {
            render_any_texture_ref(m, self.raw_models, self.textures)
        }) {
            self.counters.any_texture += 1;
            return Some(avg_colour(&tex));
        }
        None
    }
}

/// Default regex-rewrite list shared across palette commands. Generic
/// patterns (fences, gates, walls) are namespace-agnostic; minecraft-specific
/// patterns (crops at final stage, fire frame 0, …) target vanilla quirks.
pub(crate) fn default_regex_mappings() -> Vec<RegexMapping> {
    vec![
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
    ]
}
