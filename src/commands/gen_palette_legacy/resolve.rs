// Best-effort texture resolution for modded 1.7.10 blocks.
//
// The authoritative block → texture mapping for a mod lives inside its
// compiled `.class` files. Without running Minecraft we can only match on
// filenames. This module tries a handful of sensible permutations, reports
// which tier succeeded, and returns the first hit.

use super::packs::TexturePack;
use crate::anvil::legacy::palette::Rgba;
use crate::commands::gen_palette::color::avg_colour;

#[derive(Default, Debug)]
pub struct ResolveStats {
    pub vanilla: usize,
    pub modded_exact: usize,
    pub modded_fuzzy: usize,
    pub fallback: usize,
    pub missing: usize,
    pub malformed: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum MatchKind {
    Exact,
    Fuzzy,
}

pub fn resolve_modded(
    ns: &str,
    local: &str,
    packs: &[TexturePack],
) -> Option<(Rgba, MatchKind)> {
    // Mod namespaces in FML are often mixed-case (HardcoreEnderExpansion)
    // but asset directories are lowercase. Try both.
    let candidates = candidate_keys(ns, local);

    // Exact match, case-insensitive — preserve pack order via linear scan.
    for key in &candidates {
        for pack in packs {
            if let Some(tex) = pack.textures.get(key) {
                return Some((avg_colour(tex), MatchKind::Exact));
            }
            let lc = key.to_lowercase();
            if let Some(orig_key) = pack.textures_ci.get(&lc) {
                if let Some(tex) = pack.textures.get(orig_key) {
                    return Some((avg_colour(tex), MatchKind::Exact));
                }
            }
        }
    }

    // Fuzzy: scan the lowercase namespace's textures for one whose local
    // filename contains the block name fragment. Useful for blocks like
    // `gregtech:gt.blockmetal1` where the texture is actually
    // `blockmetal1.png` or `metal_item_casings.png`.
    let ns_lc = ns.to_lowercase();
    let needle = fuzzy_needle(local);
    if !needle.is_empty() {
        for pack in packs {
            for (key, tex) in &pack.textures {
                let Some((key_ns, key_rest)) = key.split_once(':') else {
                    continue;
                };
                if key_ns != ns_lc {
                    continue;
                }
                // key_rest is "block/<filename>" — extract the filename and
                // check containment (case-insensitive).
                let filename = key_rest.strip_prefix("block/").unwrap_or(key_rest);
                if filename.to_lowercase().contains(&needle) {
                    return Some((avg_colour(tex), MatchKind::Fuzzy));
                }
            }
        }
    }

    None
}

/// Candidate texture keys to try for an exact match. Ordered most-likely
/// first: the registered name as-is, then stripped common prefixes, then
/// underscore/dot normalizations.
fn candidate_keys(ns: &str, local: &str) -> Vec<String> {
    let mut out = Vec::new();
    let pushes = |v: &mut Vec<String>, s: &str| {
        v.push(format!("{}:block/{}", ns, s));
        // Some packs put textures under `textures/block/` (singular) instead
        // of `blocks/` — we normalize both to `block/` at load time, so only
        // one key format is tried.
    };

    pushes(&mut out, local);

    // Strip common "tile." prefix. Example: tile.stone → stone.
    if let Some(stripped) = local.strip_prefix("tile.") {
        pushes(&mut out, stripped);
    }

    // Many mods register blocks like `gt.blockmetal1`. The texture is usually
    // `blockmetal1.png` or `metal1.png`. Try stripping the leading segment
    // up to and including the first dot.
    if let Some(idx) = local.find('.') {
        let stripped = &local[idx + 1..];
        if !stripped.is_empty() && stripped != local {
            pushes(&mut out, stripped);
        }
    }

    // Replace dots with underscores — some mods use `foo.bar` name but
    // `foo_bar.png` texture.
    if local.contains('.') {
        let replaced = local.replace('.', "_");
        pushes(&mut out, &replaced);
    }

    out
}

/// Build a lowercase "needle" fragment to search for in texture filenames.
/// Strips common prefixes so `gt.blockmetal1` → `metal1` (the interesting
/// suffix). If the result is too short (< 3 chars) we return empty — any
/// shorter fragment matches too much noise.
fn fuzzy_needle(local: &str) -> String {
    let lc = local.to_lowercase();
    let trimmed: &str = lc
        .strip_prefix("tile.")
        .or_else(|| lc.strip_prefix("block."))
        .or_else(|| lc.strip_prefix("gt."))
        .unwrap_or(&lc);
    // Prefer the trailing segment after the last '.' if the name has dotted
    // segments.
    let needle = if let Some(idx) = trimmed.rfind('.') {
        &trimmed[idx + 1..]
    } else {
        trimmed
    };
    if needle.len() < 3 {
        String::new()
    } else {
        needle.to_string()
    }
}
