// Vanilla fallback additions + base-color backfill. Modern-path only — the
// legacy paths have their own per-version post-processing (biome tints keyed
// by numeric block id).

use fastanvil::Rgba;
use log::info;
use std::collections::HashMap;

/// Vanilla-only fallbacks for blocks the renderer can't derive a color for
/// (water, lava, air, etc.).
pub fn add_missing_blocks(palette: &mut HashMap<String, Rgba>) {
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
pub fn add_base_colors(palette: &mut HashMap<String, Rgba>) {
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
