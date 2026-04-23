// Raw model/blockstate types for fallback access.
// fastanvil's `Face.texture` field is private, so we parse the same JSON into
// our own public-field structs alongside fastanvil's types. Only used when
// `Renderer::get_top` fails or for multipart blockstates (which fastanvil
// doesn't render).

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawFace {
    pub(super) texture: String,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawElement {
    #[serde(default)]
    pub(super) faces: HashMap<String, RawFace>,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawModel {
    pub(super) parent: Option<String>,
    #[serde(default)]
    pub(super) textures: Option<HashMap<String, String>>,
    #[serde(default)]
    pub(super) elements: Option<Vec<RawElement>>,
    /// Forge custom model loaders (e.g. `functionalstorage:framedblock`) skip
    /// standard elements and put their per-face textures inside a `children`
    /// map — one inner "sub-model" per component. We only capture enough to
    /// pull texture refs out for the last-ditch any-texture fallback.
    #[serde(default)]
    pub(super) children: Option<HashMap<String, RawChild>>,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawChild {
    #[serde(default)]
    pub(super) parent: Option<String>,
    #[serde(default)]
    pub(super) textures: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawVariantRef {
    model: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(super) enum RawVariantSpec {
    Single(RawVariantRef),
    Many(Vec<RawVariantRef>),
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RawPart {
    pub(super) apply: RawVariantSpec,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub(super) enum RawBlockstate {
    Variants(HashMap<String, RawVariantSpec>),
    Multipart(Vec<RawPart>),
}

/// Qualify an unqualified resource reference with `minecraft:`. Mirrors what
/// fastanvil does internally; the vanilla convention is that bare strings in
/// parent/texture refs default to minecraft.
pub(super) fn qualify(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("minecraft:{}", name)
    }
}

/// Walk the parent chain, merging textures (child overrides parent) and
/// inheriting elements (child overrides if declared). Resolves `#ref`
/// texture variables at the end. Returns None if the root model is missing.
pub(super) fn flatten_raw_model(
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
pub(super) fn resolve_face_texture(face_tex: &str, model: &RawModel) -> Option<String> {
    let resolved = if let Some(key) = face_tex.strip_prefix('#') {
        model.textures.as_ref()?.get(key)?.clone()
    } else {
        face_tex.to_string()
    };
    Some(qualify(&resolved))
}

/// Given a raw variant spec, pick the first model name. Variants::Many just
/// picks element 0 (vanilla would pick by weight, but we only need color
/// and variants are visually similar).
pub(super) fn first_model_name(spec: &RawVariantSpec) -> Option<&str> {
    match spec {
        RawVariantSpec::Single(v) => Some(&v.model),
        RawVariantSpec::Many(vs) => vs.first().map(|v| v.model.as_str()),
    }
}
