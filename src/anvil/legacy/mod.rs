// Pre-1.13 chunk handling (1.7.10 + NotEnoughIDs).
//
// Vanilla 1.7.10 stores block IDs as `Blocks` (u8) plus an optional `Add`
// nibble array for the top 4 bits (12-bit IDs, 0..4095). NotEnoughIDs replaces
// those with `Blocks16` — 4096 big-endian u16s — and extends metadata the same
// way via `Data16`. This module parses both layouts.
//
// Region-file framing is unchanged from 1.13+, so we keep using
// `fastanvil::Region` for I/O and only introduce a new NBT parser here.

pub mod chunk;
pub mod palette;
pub mod render;

pub use palette::LegacyPalette;
pub use render::{LegacyTopShadeRenderer, render_legacy_region};
