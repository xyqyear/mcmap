// Vanilla 1.7.10 / 1.12.2 (block name, meta) → texture-path table, plus the
// biome-tint + water/lava/air post-processing that both legacy palette
// pipelines apply after resolution.
//
// 1.7.10 has no blockstate/model JSONs — the mapping from numeric ID +
// metadata to a texture is encoded in Java source inside the `Block`
// subclasses. Texture filenames under `assets/minecraft/textures/blocks/`
// are stable between 1.7.10 and 1.12.2; only the directory plural ("blocks/"
// vs "block/") shifted in 1.13, so the same table drives both legacy pipelines.
//
// Keys are the block's unlocalized/registry name *without* the `minecraft:`
// prefix. Values are `(meta, texture_key)` pairs where `texture_key` is the
// pack-internal key — always `"minecraft:block/<filename_without_png>"`.
//
// When a block only has one face that matters for top-down rendering, a single
// `(0, path)` is emitted — variants that differ only in side appearance reuse
// the same top texture.

use fastanvil::Rgba;
use std::collections::HashMap;
use std::hash::Hash;

/// (meta, texture_key) variants for a vanilla block. Empty vec means "not in
/// table" — caller falls back to filename probing.
pub fn variants_for(name: &str) -> Vec<(u16, &'static str)> {
    match name {
        "air" => return vec![],
        "stone" => return single("minecraft:block/stone"),
        "grass" => return single("minecraft:block/grass_top"),
        "dirt" => {
            return vec![
                (0, "minecraft:block/dirt"),
                (1, "minecraft:block/dirt"),            // coarse (= dirt)
                (2, "minecraft:block/dirt_podzol_top"), // podzol
            ];
        }
        "cobblestone" => return single("minecraft:block/cobblestone"),
        "planks" => {
            return vec![
                (0, "minecraft:block/planks_oak"),
                (1, "minecraft:block/planks_spruce"),
                (2, "minecraft:block/planks_birch"),
                (3, "minecraft:block/planks_jungle"),
                (4, "minecraft:block/planks_acacia"),
                (5, "minecraft:block/planks_big_oak"),
            ];
        }
        "sapling" => {
            return vec![
                (0, "minecraft:block/sapling_oak"),
                (1, "minecraft:block/sapling_spruce"),
                (2, "minecraft:block/sapling_birch"),
                (3, "minecraft:block/sapling_jungle"),
                (4, "minecraft:block/sapling_acacia"),
                (5, "minecraft:block/sapling_roofed_oak"),
            ];
        }
        "bedrock" => return single("minecraft:block/bedrock"),
        "flowing_water" | "water" => return single("minecraft:block/water_still"),
        "flowing_lava" | "lava" => return single("minecraft:block/lava_still"),
        "sand" => {
            return vec![
                (0, "minecraft:block/sand"),
                (1, "minecraft:block/red_sand"),
            ];
        }
        "gravel" => return single("minecraft:block/gravel"),
        "gold_ore" => return single("minecraft:block/gold_ore"),
        "iron_ore" => return single("minecraft:block/iron_ore"),
        "coal_ore" => return single("minecraft:block/coal_ore"),
        "log" => {
            // meta & 0x3 selects species; high bits are axis (irrelevant for
            // top-down rendering — the top face is what shows through).
            return vec![
                (0, "minecraft:block/log_oak_top"),
                (1, "minecraft:block/log_spruce_top"),
                (2, "minecraft:block/log_birch_top"),
                (3, "minecraft:block/log_jungle_top"),
            ];
        }
        "leaves" => {
            return vec![
                (0, "minecraft:block/leaves_oak"),
                (1, "minecraft:block/leaves_spruce"),
                (2, "minecraft:block/leaves_birch"),
                (3, "minecraft:block/leaves_jungle"),
            ];
        }
        "sponge" => return single("minecraft:block/sponge"),
        "glass" => return single("minecraft:block/glass"),
        "lapis_ore" => return single("minecraft:block/lapis_ore"),
        "lapis_block" => return single("minecraft:block/lapis_block"),
        "dispenser" => return single("minecraft:block/furnace_top"),
        "sandstone" => {
            return vec![
                (0, "minecraft:block/sandstone_top"),
                (1, "minecraft:block/sandstone_top"),
                (2, "minecraft:block/sandstone_top"),
            ];
        }
        "noteblock" => return single("minecraft:block/noteblock"),
        "bed" => return single("minecraft:block/bed_head_top"),
        "golden_rail" => return single("minecraft:block/rail_golden"),
        "detector_rail" => return single("minecraft:block/rail_detector"),
        "sticky_piston" => return single("minecraft:block/piston_top_sticky"),
        "web" => return single("minecraft:block/web"),
        "tallgrass" => {
            return vec![
                (0, "minecraft:block/deadbush"),
                (1, "minecraft:block/tallgrass"),
                (2, "minecraft:block/fern"),
            ];
        }
        "deadbush" => return single("minecraft:block/deadbush"),
        "piston" => return single("minecraft:block/piston_top_normal"),
        "piston_head" | "piston_extension" => return vec![], // invisible
        "wool" => return wool_like("minecraft:block/wool_colored_"),
        "yellow_flower" => return single("minecraft:block/flower_dandelion"),
        "red_flower" => {
            return vec![
                (0, "minecraft:block/flower_rose"),
                (1, "minecraft:block/flower_blue_orchid"),
                (2, "minecraft:block/flower_allium"),
                (3, "minecraft:block/flower_houstonia"),
                (4, "minecraft:block/flower_tulip_red"),
                (5, "minecraft:block/flower_tulip_orange"),
                (6, "minecraft:block/flower_tulip_white"),
                (7, "minecraft:block/flower_tulip_pink"),
                (8, "minecraft:block/flower_oxeye_daisy"),
            ];
        }
        "brown_mushroom" => return single("minecraft:block/mushroom_brown"),
        "red_mushroom" => return single("minecraft:block/mushroom_red"),
        "gold_block" => return single("minecraft:block/gold_block"),
        "iron_block" => return single("minecraft:block/iron_block"),
        "double_stone_slab" | "stone_slab" => {
            return vec![
                (0, "minecraft:block/stone_slab_top"),
                (1, "minecraft:block/sandstone_top"),
                (2, "minecraft:block/planks_oak"),
                (3, "minecraft:block/cobblestone"),
                (4, "minecraft:block/brick"),
                (5, "minecraft:block/stonebrick"),
                (6, "minecraft:block/nether_brick"),
                (7, "minecraft:block/quartz_block_top"),
            ];
        }
        "brick_block" => return single("minecraft:block/brick"),
        "tnt" => return single("minecraft:block/tnt_top"),
        "bookshelf" => return single("minecraft:block/planks_oak"),
        "mossy_cobblestone" => return single("minecraft:block/cobblestone_mossy"),
        "obsidian" => return single("minecraft:block/obsidian"),
        "torch" => return single("minecraft:block/torch_on"),
        "fire" => return single("minecraft:block/fire_layer_0"),
        "mob_spawner" => return single("minecraft:block/mob_spawner"),
        "oak_stairs" => return single("minecraft:block/planks_oak"),
        "chest" => return single("minecraft:block/planks_oak"),
        "redstone_wire" => return single("minecraft:block/redstone_dust_line"),
        "diamond_ore" => return single("minecraft:block/diamond_ore"),
        "diamond_block" => return single("minecraft:block/diamond_block"),
        "crafting_table" => return single("minecraft:block/crafting_table_top"),
        "wheat" => return single("minecraft:block/wheat_stage_7"),
        "farmland" => return single("minecraft:block/farmland_dry"),
        "furnace" | "lit_furnace" => return single("minecraft:block/furnace_top"),
        "standing_sign" | "wall_sign" => return single("minecraft:block/planks_oak"),
        "wooden_door" => return single("minecraft:block/door_wood_upper"),
        "ladder" => return single("minecraft:block/ladder"),
        "rail" => return single("minecraft:block/rail_normal"),
        "stone_stairs" => return single("minecraft:block/cobblestone"),
        "lever" => return vec![],
        "stone_pressure_plate" => return single("minecraft:block/stone"),
        "iron_door" => return single("minecraft:block/door_iron_upper"),
        "wooden_pressure_plate" => return single("minecraft:block/planks_oak"),
        "redstone_ore" | "lit_redstone_ore" => return single("minecraft:block/redstone_ore"),
        "unlit_redstone_torch" => return single("minecraft:block/redstone_torch_off"),
        "redstone_torch" => return single("minecraft:block/redstone_torch_on"),
        "stone_button" => return single("minecraft:block/stone"),
        "snow_layer" | "snow" => return single("minecraft:block/snow"),
        "ice" => return single("minecraft:block/ice"),
        "cactus" => return single("minecraft:block/cactus_top"),
        "clay" => return single("minecraft:block/clay"),
        "reeds" => return single("minecraft:block/reeds"),
        "jukebox" => return single("minecraft:block/jukebox_top"),
        "fence" => return single("minecraft:block/planks_oak"),
        "pumpkin" | "lit_pumpkin" => return single("minecraft:block/pumpkin_top"),
        "netherrack" => return single("minecraft:block/netherrack"),
        "soul_sand" => return single("minecraft:block/soul_sand"),
        "glowstone" => return single("minecraft:block/glowstone"),
        "portal" | "end_portal" => return single("minecraft:block/portal"),
        "cake" => return single("minecraft:block/cake_top"),
        "unpowered_repeater" => return single("minecraft:block/repeater_off"),
        "powered_repeater" => return single("minecraft:block/repeater_on"),
        "stained_glass" => return wool_like("minecraft:block/glass_"),
        "trapdoor" => return single("minecraft:block/trapdoor"),
        "monster_egg" => {
            return vec![
                (0, "minecraft:block/stone"),
                (1, "minecraft:block/cobblestone"),
                (2, "minecraft:block/stonebrick"),
                (3, "minecraft:block/stonebrick_mossy"),
                (4, "minecraft:block/stonebrick_cracked"),
                (5, "minecraft:block/stonebrick_carved"),
            ];
        }
        "stonebrick" => {
            return vec![
                (0, "minecraft:block/stonebrick"),
                (1, "minecraft:block/stonebrick_mossy"),
                (2, "minecraft:block/stonebrick_cracked"),
                (3, "minecraft:block/stonebrick_carved"),
            ];
        }
        "brown_mushroom_block" => return single("minecraft:block/mushroom_block_skin_brown"),
        "red_mushroom_block" => return single("minecraft:block/mushroom_block_skin_red"),
        "iron_bars" => return single("minecraft:block/iron_bars"),
        "glass_pane" => return single("minecraft:block/glass"),
        "melon_block" => return single("minecraft:block/melon_top"),
        "pumpkin_stem" => return single("minecraft:block/pumpkin_stem_connected"),
        "melon_stem" => return single("minecraft:block/melon_stem_connected"),
        "vine" => return single("minecraft:block/vine"),
        "fence_gate" => return single("minecraft:block/planks_oak"),
        "brick_stairs" => return single("minecraft:block/brick"),
        "stone_brick_stairs" => return single("minecraft:block/stonebrick"),
        "mycelium" => return single("minecraft:block/mycelium_top"),
        "waterlily" => return single("minecraft:block/waterlily"),
        "nether_brick" | "nether_brick_fence" | "nether_brick_stairs" => {
            return single("minecraft:block/nether_brick");
        }
        "nether_wart" => return single("minecraft:block/nether_wart_stage_2"),
        "enchanting_table" => return single("minecraft:block/enchanting_table_top"),
        "brewing_stand" => return single("minecraft:block/brewing_stand_base"),
        "cauldron" => return single("minecraft:block/cauldron_top"),
        "end_portal_frame" => return single("minecraft:block/endframe_top"),
        "end_stone" => return single("minecraft:block/end_stone"),
        "dragon_egg" => return single("minecraft:block/dragon_egg"),
        "redstone_lamp" => return single("minecraft:block/redstone_lamp_off"),
        "lit_redstone_lamp" => return single("minecraft:block/redstone_lamp_on"),
        "double_wooden_slab" | "wooden_slab" => {
            return vec![
                (0, "minecraft:block/planks_oak"),
                (1, "minecraft:block/planks_spruce"),
                (2, "minecraft:block/planks_birch"),
                (3, "minecraft:block/planks_jungle"),
                (4, "minecraft:block/planks_acacia"),
                (5, "minecraft:block/planks_big_oak"),
            ];
        }
        "cocoa" => return single("minecraft:block/cocoa_stage_2"),
        "sandstone_stairs" => return single("minecraft:block/sandstone_top"),
        "emerald_ore" => return single("minecraft:block/emerald_ore"),
        "ender_chest" => return single("minecraft:block/obsidian"),
        "tripwire_hook" | "tripwire" => return vec![],
        "emerald_block" => return single("minecraft:block/emerald_block"),
        "spruce_stairs" => return single("minecraft:block/planks_spruce"),
        "birch_stairs" => return single("minecraft:block/planks_birch"),
        "jungle_stairs" => return single("minecraft:block/planks_jungle"),
        "command_block" => return single("minecraft:block/command_block"),
        "beacon" => return single("minecraft:block/beacon"),
        "cobblestone_wall" => {
            return vec![
                (0, "minecraft:block/cobblestone"),
                (1, "minecraft:block/cobblestone_mossy"),
            ];
        }
        "flower_pot" => return single("minecraft:block/flower_pot"),
        "carrots" => return single("minecraft:block/carrots_stage_3"),
        "potatoes" => return single("minecraft:block/potatoes_stage_3"),
        "wooden_button" => return single("minecraft:block/planks_oak"),
        "skull" => return vec![],
        "anvil" => return single("minecraft:block/anvil_top_damaged_0"),
        "trapped_chest" => return single("minecraft:block/planks_oak"),
        "light_weighted_pressure_plate" => return single("minecraft:block/gold_block"),
        "heavy_weighted_pressure_plate" => return single("minecraft:block/iron_block"),
        "unpowered_comparator" => return single("minecraft:block/comparator_off"),
        "powered_comparator" => return single("minecraft:block/comparator_on"),
        "daylight_detector" => return single("minecraft:block/daylight_detector_top"),
        "redstone_block" => return single("minecraft:block/redstone_block"),
        "quartz_ore" => return single("minecraft:block/quartz_ore"),
        "hopper" => return single("minecraft:block/hopper_top"),
        "quartz_block" => return single("minecraft:block/quartz_block_top"),
        "quartz_stairs" => return single("minecraft:block/quartz_block_top"),
        "activator_rail" => return single("minecraft:block/rail_activator"),
        "dropper" => return single("minecraft:block/furnace_top"),
        "stained_hardened_clay" => return wool_like("minecraft:block/hardened_clay_stained_"),
        "stained_glass_pane" => return wool_like("minecraft:block/glass_"),
        "leaves2" => {
            return vec![
                (0, "minecraft:block/leaves_acacia"),
                (1, "minecraft:block/leaves_big_oak"),
            ];
        }
        "log2" => {
            return vec![
                (0, "minecraft:block/log_acacia_top"),
                (1, "minecraft:block/log_big_oak_top"),
            ];
        }
        "acacia_stairs" => return single("minecraft:block/planks_acacia"),
        "dark_oak_stairs" => return single("minecraft:block/planks_big_oak"),
        "hay_block" => return single("minecraft:block/hay_block_top"),
        "carpet" => return wool_like("minecraft:block/wool_colored_"),
        "hardened_clay" => return single("minecraft:block/hardened_clay"),
        "coal_block" => return single("minecraft:block/coal_block"),
        "packed_ice" => return single("minecraft:block/ice_packed"),
        "double_plant" => {
            return vec![
                (0, "minecraft:block/double_plant_sunflower_top"),
                (1, "minecraft:block/double_plant_syringa_top"),
                (2, "minecraft:block/double_plant_grass_top"),
                (3, "minecraft:block/double_plant_fern_top"),
                (4, "minecraft:block/double_plant_rose_top"),
                (5, "minecraft:block/double_plant_paeonia_top"),
            ];
        }
        _ => vec![],
    }
}

fn single(path: &'static str) -> Vec<(u16, &'static str)> {
    vec![(0, path)]
}

/// Standard 16-color wool/carpet/stained-glass/hardened-clay variant family.
/// Meta 0..15 → white, orange, magenta, light_blue, yellow, lime, pink, gray,
/// silver, cyan, purple, blue, brown, green, red, black.
fn wool_like(prefix: &'static str) -> Vec<(u16, &'static str)> {
    const COLORS: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "silver",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    // Leaking static strings once per variant is acceptable — the table is
    // static and this only runs during palette generation (one-shot).
    COLORS
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let s: &'static str = Box::leak(format!("{}{}", prefix, c).into_boxed_str());
            (i as u16, s)
        })
        .collect()
}

/// Apply biome tints + water/lava/air special-casing to vanilla-namespaced
/// blocks. Shared by both legacy pipelines — runs before user overrides so
/// the user can still override these.
pub fn apply_vanilla_postprocess<I>(
    palette: &mut HashMap<String, Rgba>,
    id_to_name: &HashMap<I, String>,
) where
    I: Copy + std::fmt::Display + Eq + Hash,
{
    // Tints applied to every `id|meta` + bare `id` entry of the block.
    let grass_tint: Rgba = [124, 189, 107, 255];
    let foliage_tint: Rgba = [84, 130, 54, 255];
    let vine_tint: Rgba = [106, 136, 44, 200];

    for (id, name) in id_to_name {
        let (_ns, local) = match name.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        match local {
            "air" => set_block_color(palette, *id, [0, 0, 0, 0]),
            "water" | "flowing_water" => set_block_color(palette, *id, [63, 118, 228, 180]),
            "lava" | "flowing_lava" => set_block_color(palette, *id, [207, 78, 0, 255]),
            "grass" | "mycelium" => multiply_block_color(palette, *id, grass_tint),
            "tallgrass" | "fern" | "double_plant" => {
                multiply_block_color(palette, *id, grass_tint)
            }
            "leaves" | "leaves2" | "waterlily" => {
                multiply_block_color(palette, *id, foliage_tint)
            }
            "vine" => multiply_block_color(palette, *id, vine_tint),
            _ => {}
        }
    }
}

fn set_block_color<I: std::fmt::Display>(palette: &mut HashMap<String, Rgba>, id: I, color: Rgba) {
    let prefix_eq = format!("{}", id);
    let prefix_pipe = format!("{}|", id);
    let mut matching: Vec<String> = palette
        .keys()
        .filter(|k| *k == &prefix_eq || k.starts_with(&prefix_pipe))
        .cloned()
        .collect();
    matching.push(prefix_eq);
    for k in matching {
        palette.insert(k, color);
    }
}

fn multiply_block_color<I: std::fmt::Display>(
    palette: &mut HashMap<String, Rgba>,
    id: I,
    tint: Rgba,
) {
    let prefix_eq = format!("{}", id);
    let prefix_pipe = format!("{}|", id);
    let keys: Vec<String> = palette
        .keys()
        .filter(|k| *k == &prefix_eq || k.starts_with(&prefix_pipe))
        .cloned()
        .collect();
    for k in keys {
        if let Some(existing) = palette.get(&k).copied() {
            palette.insert(k, multiply_rgba(existing, tint));
        }
    }
}

fn multiply_rgba(a: Rgba, b: Rgba) -> Rgba {
    [
        mul_channel(a[0], b[0]),
        mul_channel(a[1], b[1]),
        mul_channel(a[2], b[2]),
        mul_channel(a[3], b[3]),
    ]
}

#[inline]
fn mul_channel(a: u8, b: u8) -> u8 {
    (((a as u16) * (b as u16)) / 255) as u8
}
