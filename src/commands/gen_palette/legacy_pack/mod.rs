// Texture-only resource-pack loader + fuzzy modded resolver.
//
// Used exclusively by `gen-palette legacy` (1.7.10). 1.7.10 packs don't ship
// blockstate/model JSONs (block rendering is hard-coded in Java), just raw
// PNGs under `assets/<ns>/textures/blocks/`. So we index textures only —
// cheaper, simpler, and avoids pulling unrelated JSON parse errors into the
// path.

pub mod packs;
pub mod resolve;

pub use packs::{TexturePack, load_texture_packs};
pub use resolve::{MatchKind, ResolveStats, resolve_modded};
