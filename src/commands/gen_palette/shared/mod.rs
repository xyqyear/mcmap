// Shared helpers used by every palette-gen variant.
//
//   - `color` — texture → single RGBA averaging.
//   - `overrides` — load/apply user color overrides (last word on any key).
//   - `vanilla_1x` — hand-curated (name, meta) → texture_path table covering
//     ~100 vanilla 1.x blocks + biome-tint / water-lava post-processing. Used
//     by both legacy pipelines (1.7.10 and Forge 1.12.2 REI).

pub mod color;
pub mod overrides;
pub mod progress;
pub mod vanilla_1x;
