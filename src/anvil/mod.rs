// Anvil-format rendering.
//
// The on-disk region file layout (.mca container, zlib-framed chunks, 32×32
// grid) is unchanged across all supported Minecraft versions and lives in
// `region`. Per-version chunk decoding + column rendering is a `RenderEngine`
// that plugs into the shared `pipeline`:
//
//   - `modern` — 1.13+ via `fastanvil::JavaChunk` + `TopShadeRenderer`.
//   - `legacy` — pre-1.13 (vanilla 1.7.10/1.12.2, NEID, Forge 1.12.2 REI).
//
// `palette` handles the top-level load — its `AnyPalette` carries enough to
// build the right engine at render time.

pub mod legacy;
pub mod modern;
pub mod palette;
pub mod pipeline;
pub mod region;

pub use palette::AnyPalette;
pub use pipeline::{RegionMap, RenderEngine, Rgba, render_region};
pub use region::{CCoord, RCoord, RegionFileLoader};

// Re-exports for common modern-path usage from other commands.
pub use modern::{HeightMode, RenderedPalette, TopShadeRenderer};
