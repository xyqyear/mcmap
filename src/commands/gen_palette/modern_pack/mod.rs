// Modern resource-pack loader + tiered blockstate resolver.
//
// Used by both the `gen-palette modern` (1.13+) and `gen-palette forge112`
// (1.12.2 REI) subcommands — 1.12.2 already ships blockstate/model JSONs, so
// the same pipeline resolves modded blocks on both.
//
// Treats vanilla and modded jars identically: every pack is a zip archive
// containing `assets/<namespace>/{blockstates,models,textures}/...`. Namespace
// is derived from the path, never hardcoded.
//
// Resolution tiers (first success wins, see `resolve::Resolver`):
//   0. fastanvil renderer — blockstate variant → model → top face texture.
//   1. raw-model side-face fallback.
//   1.5 particle-texture fallback (for tile-entity-rendered blocks).
//   1.7 any-texture-reference in the model tree (Forge custom loaders).
//   2. regex rewrites (generic + vanilla quirks).
//   3. direct texture-path probe.
//   4. substring match across namespace.
//   5. generic sibling blockstate bridge (for dynamic-name registries).

pub mod packs;
pub mod postprocess;
pub mod raw;
pub mod resolve;

pub use packs::{Pools, load_packs};
pub use postprocess::{add_base_colors, add_missing_blocks};
pub use resolve::{Counters, Resolver, default_regex_mappings};
