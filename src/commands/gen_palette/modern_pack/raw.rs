// Raw model/blockstate types for fallback access.
// fastanvil's `Face.texture` field is private, so we parse the same JSON into
// our own public-field structs alongside fastanvil's types. Only used when
// `Renderer::get_top` fails or for multipart blockstates (which fastanvil
// doesn't render).

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Debug, Clone)]
pub struct RawFace {
    pub texture: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawElement {
    #[serde(default)]
    pub faces: HashMap<String, RawFace>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawModel {
    pub parent: Option<String>,
    #[serde(default)]
    pub textures: Option<HashMap<String, String>>,
    #[serde(default)]
    pub elements: Option<Vec<RawElement>>,
    /// Forge custom model loaders (e.g. `functionalstorage:framedblock`) skip
    /// standard elements and put their per-face textures inside a `children`
    /// map — one inner "sub-model" per component. We only capture enough to
    /// pull texture refs out for the last-ditch any-texture fallback.
    #[serde(default)]
    pub children: Option<HashMap<String, RawChild>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawChild {
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub textures: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawVariantRef {
    pub model: String,
    /// Forge-blockstate-only: per-variant `textures` overrides that should be
    /// merged onto the flattened model's texture map (variant wins). Vanilla
    /// blockstates leave this `None`. Synthesized during Forge parsing —
    /// vanilla deserialization just ignores the field.
    #[serde(default, skip)]
    pub textures: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum RawVariantSpec {
    Single(RawVariantRef),
    Many(Vec<RawVariantRef>),
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawPart {
    pub apply: RawVariantSpec,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum RawBlockstate {
    Variants(HashMap<String, RawVariantSpec>),
    Multipart(Vec<RawPart>),
}

/// 1.12.2's Forge blockstate variant — `{"forge_marker": 1, "defaults": {...},
/// "variants": {...}}`. Distinct enough from the vanilla shape that the same
/// serde enum can't cover both, so we parse it separately and convert.
///
/// Variants are stored opaquely as `serde_json::Value` because the on-disk
/// shape is highly polymorphic (direct leaf, list of leaves, nested
/// property-name → value-name → leaf, combinatorial keys like
/// `"facing=north,powered=true"`). We walk the value tree manually in
/// `parse_blockstate_lenient` to extract every leaf — each leaf becomes a
/// `RawVariantRef` with its `textures` overrides merged on top of
/// `defaults.textures` (variant wins).
#[derive(Deserialize, Debug)]
struct ForgeBlockstate {
    /// Always 1 for the format we recognize. Failure to match is the signal
    /// to skip this parser entirely.
    forge_marker: i32,
    #[serde(default)]
    defaults: Option<ForgeDefaults>,
    #[serde(default)]
    variants: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Deserialize, Debug)]
struct ForgeDefaults {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    textures: Option<HashMap<String, String>>,
}

/// Try the standard format first, then the Forge-marker-1 fallback. Returns
/// `None` when both fail — which usually means the JSON is malformed (or
/// uses an even more obscure custom format).
pub fn parse_blockstate_lenient(bytes: &[u8]) -> Option<RawBlockstate> {
    if let Ok(bs) = serde_json::from_slice::<RawBlockstate>(bytes) {
        return Some(bs);
    }
    let forge: ForgeBlockstate = serde_json::from_slice(bytes).ok()?;
    if forge.forge_marker != 1 {
        return None;
    }
    let default_model = forge.defaults.as_ref().and_then(|d| d.model.clone());
    let default_textures = forge
        .defaults
        .as_ref()
        .and_then(|d| d.textures.clone())
        .unwrap_or_default();

    // Walk the variants tree (values can be deeply nested by property names
    // before reaching a leaf). Collect every leaf as (model_override?,
    // textures_override?). If no variants block exists, synthesize a single
    // empty leaf so the default model gets a chance.
    let mut leaves: Vec<(Option<String>, HashMap<String, String>)> = Vec::new();
    if let Some(forge_vars) = &forge.variants {
        for (key, value) in forge_vars {
            if key == "inventory" {
                continue;
            }
            collect_forge_leaves(value, &mut leaves);
        }
    }
    if leaves.is_empty() {
        leaves.push((None, HashMap::new()));
    }

    let mut variants_out: HashMap<String, RawVariantSpec> = HashMap::new();
    let mut idx = 0usize;
    for (leaf_model, leaf_textures) in leaves {
        let Some(model) = leaf_model.or_else(|| default_model.clone()) else {
            continue;
        };
        let mut merged = default_textures.clone();
        for (k, v) in leaf_textures {
            merged.insert(k, v);
        }
        let textures = if merged.is_empty() { None } else { Some(merged) };
        let key = format!("__forge_{}", idx);
        idx += 1;
        variants_out.insert(
            key,
            RawVariantSpec::Single(RawVariantRef { model, textures }),
        );
    }
    if variants_out.is_empty() {
        return None;
    }
    Some(RawBlockstate::Variants(variants_out))
}

/// Recursively walk a Forge variants subtree, collecting every "leaf" — an
/// object that names at least one of `model` / `textures`. Lists are
/// flat-mapped. Bare property-name maps (whose values are themselves
/// variant-key maps) recurse without producing a leaf themselves.
///
/// Also recurses into `submodel` — Forge's combinatorial sub-rendering
/// mechanism (one named cube per submodel, all stacked at the same world
/// position; common in EnderIO's IO-mode overlays). Each submodel entry is
/// itself a leaf and frequently carries the only model/texture pair worth
/// rendering when the parent only sets `textures`.
fn collect_forge_leaves(
    value: &serde_json::Value,
    out: &mut Vec<(Option<String>, HashMap<String, String>)>,
) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let has_leaf_field = map.contains_key("model") || map.contains_key("textures");
            if has_leaf_field {
                let model = map.get("model").and_then(|v| v.as_str()).map(String::from);
                let textures = map
                    .get("textures")
                    .and_then(|v| v.as_object())
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect::<HashMap<_, _>>()
                    })
                    .unwrap_or_default();
                out.push((model, textures));
                if let Some(sub) = map.get("submodel") {
                    collect_forge_leaves(sub, out);
                }
            } else {
                for v in map.values() {
                    collect_forge_leaves(v, out);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_forge_leaves(v, out);
            }
        }
        _ => {}
    }
}

/// Qualify an unqualified resource reference with `minecraft:`. Mirrors what
/// fastanvil does internally; vanilla convention is that bare strings default
/// to minecraft.
pub fn qualify(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("minecraft:{}", name)
    }
}

/// Like `qualify`, but also adds the `block/` directory prefix when missing.
/// Forge 1.12.2 mod blockstates routinely reference models as
/// `"<ns>:my_block"` or even `"<ns>:fission_shield/boron_silver_off"` (no
/// `block/`) — vanilla's resolver implicitly looks under `block/` for models
/// referenced from a blockstate, and fastanvil's `qualify` doesn't replicate
/// that. Detect by absence of the `block/` / `item/` prefix on the path
/// portion (so `block/<sub>/<sub>` paths from model parents are preserved).
/// Use this for *model* lookups; texture references keep their full
/// path-with-subfolder convention.
pub fn qualify_model(name: &str) -> String {
    let qualified = qualify(name);
    let (ns, rest) = qualified.split_once(':').unwrap_or(("minecraft", &qualified));
    if rest.starts_with("block/") || rest.starts_with("item/") {
        qualified
    } else {
        format!("{}:block/{}", ns, rest)
    }
}

/// Walk the parent chain, merging textures (child overrides parent) and
/// inheriting elements (child overrides if declared). Resolves `#ref`
/// texture variables at the end. Returns None if the root model is missing.
pub fn flatten_raw_model(
    name: &str,
    raw_models: &HashMap<String, RawModel>,
) -> Option<RawModel> {
    flatten_raw_model_with_overrides(name, raw_models, None)
}

/// Same as `flatten_raw_model` but applies an extra texture map *after* the
/// parent chain merge but *before* `#ref` resolution. Used for Forge-format
/// blockstates whose `defaults.textures` / per-variant `textures` override
/// the model's own texture vars.
pub fn flatten_raw_model_with_overrides(
    name: &str,
    raw_models: &HashMap<String, RawModel>,
    extra_textures: Option<&HashMap<String, String>>,
) -> Option<RawModel> {
    let mut chain: Vec<RawModel> = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(qualify_model(name));

    while let Some(key) = cur {
        if !seen.insert(key.clone()) {
            break; // cycle
        }
        let Some(m) = raw_models.get(&key) else {
            break;
        };
        chain.push(m.clone());
        cur = m.parent.as_ref().map(|p| qualify_model(p));
    }

    let mut out = chain.pop()?; // root-most ancestor
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

    if let Some(extra) = extra_textures {
        let pt = out.textures.get_or_insert_with(HashMap::new);
        for (k, v) in extra {
            pt.insert(k.clone(), v.clone());
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
pub fn resolve_face_texture(face_tex: &str, model: &RawModel) -> Option<String> {
    let resolved = if let Some(key) = face_tex.strip_prefix('#') {
        model.textures.as_ref()?.get(key)?.clone()
    } else {
        face_tex.to_string()
    };
    Some(qualify(&resolved))
}
