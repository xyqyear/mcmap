// Pre-1.13 chunk path.
//
// Two on-disk shapes land in the same in-memory `LegacyChunkData` —
// per-block (id, meta) pairs — decoded by either:
//
//   - `chunk` — vanilla 1.7.10 / 1.12.2, optionally with NEID (`Blocks16` /
//     `Data16` for 16-bit ids/meta).
//   - `chunk_forge112` — Forge 1.12.2 + RoughlyEnoughIDs / JustEnoughIDs
//     (per-section `Palette` int-array). See `docs/forge_1_12_2_rei.md`.
//
// The renderer picks the decoder at engine construction time based on the
// palette's declared format.

pub mod chunk;
pub mod chunk_forge112;
pub mod palette;
pub mod render;

pub use palette::LegacyPalette;
pub use render::LegacyTopShadeRenderer;
