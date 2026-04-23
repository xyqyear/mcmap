// 1.13+ chunk path. Delegates to `fastanvil::JavaChunk` for NBT parsing and
// `fastanvil::TopShadeRenderer` for per-chunk shading.

pub mod chunk;
pub mod render;

pub use chunk::{ChunkData, HeightMode};
pub use render::{RenderedPalette, TopShadeRenderer};
