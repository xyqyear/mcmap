// Anvil format support for overhead map rendering.
//
// Two paths exist side by side:
//
// - The 1.13+ path (`chunk`, `render`) wraps `fastanvil::JavaChunk` and its
//   rendering helpers. This is the fast, mature path for modern worlds.
// - The pre-1.13 path (`legacy`) handles 1.7.10 worlds with or without
//   NotEnoughIDs installed. Uses `fastnbt` directly for chunk parsing.
//
// Region-file I/O (`region`) is shared — the .mca container is unchanged
// between versions.

pub mod chunk;
pub mod legacy;
pub mod region;
mod render;

pub use chunk::HeightMode;
pub use region::{CCoord, RCoord, RegionFileLoader};
pub use render::{RegionMap, RenderedPalette, Rgba, TopShadeRenderer, render_region};
