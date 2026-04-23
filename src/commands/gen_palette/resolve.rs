use fastanvil::tex::Texture;
use regex::Regex;
use std::collections::HashMap;

use super::raw::{
    RawBlockstate, RawModel, first_model_name, flatten_raw_model, qualify, resolve_face_texture,
};

/// Pick the first face in a flattened model whose texture is present.
/// Preference order: up → down → side faces (block top is what matters most
/// for a top-down map; down handles blocks only visible from underneath;
/// sides are last resort).
pub(super) fn render_any_face(
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
pub(super) fn render_particle_texture(
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
pub(super) fn render_any_texture_ref(
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
pub(super) fn render_any_variant_of_block(
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
/// `mymod:steel_block` → try `mymod:block/steel_block`. Useful for mods whose
/// blockstate/model JSONs are broken or unconventional but whose textures
/// follow the standard layout.
pub(super) fn probe_texture_by_name(
    block_name: &str,
    textures: &HashMap<String, Texture>,
) -> Option<Texture> {
    let (ns, name) = block_name.split_once(':')?;
    let candidate = format!("{}:block/{}", ns, name);
    textures.get(&candidate).cloned()
}

/// Per-tier success counters. Used only for the final resolution summary.
#[derive(Default)]
pub(super) struct Counters {
    pub(super) rendered: usize,
    pub(super) side_fallback: usize,
    pub(super) particle: usize,
    pub(super) any_texture: usize,
    pub(super) mapped: usize,
    pub(super) probed: usize,
}

#[derive(Debug)]
pub(super) struct RegexMapping {
    pub(super) blockstate: Regex,
    pub(super) texture_template: &'static str,
}

impl RegexMapping {
    pub(super) fn apply(&self, blockstate: &str) -> Option<String> {
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
