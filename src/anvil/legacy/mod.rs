// Pre-1.13 chunk handling.
//
// Two on-disk formats land in the same in-memory shape (`LegacyChunkData` —
// per-block (id, meta) pairs):
//
//   - `chunk` — vanilla 1.7.10 / 1.12.2 + NotEnoughIDs (NEID). Block IDs come
//     from `Blocks` (u8) plus an optional `Add` nibble (12-bit IDs); NEID's
//     `Blocks16` / `Data16` (16-bit IDs/meta) take precedence when present.
//   - `chunk_forge112` — Forge 1.12.2 with RoughlyEnoughIDs / JustEnoughIDs.
//     Adds a per-section `Palette` int-array; `Blocks` and `Data` carry the
//     high-8 / low-4 bits of an index into that palette. See
//     `docs/forge_1_12_2_rei.md`.
//
// Region-file framing is unchanged across all of these, so I/O stays on
// `fastanvil::Region`.

pub mod chunk;
pub mod chunk_forge112;
pub mod palette;
pub mod render;

pub use palette::LegacyPalette;
pub use render::{LegacyTopShadeRenderer, render_legacy_region};
