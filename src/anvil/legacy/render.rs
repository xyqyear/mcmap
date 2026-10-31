// Top-down renderer for pre-1.13 chunks.
//
// Approach follows the mcasaenk model (see docs/mcasaenk.md §4, §10):
//
//   1. Start at the highest opaque block Y (from `HeightMap` if present,
//      otherwise scan from top of world).
//   2. Walk down compositing each block's palette color until alpha reaches
//      full — this lets us render through glass, leaves, and water without
//      special-casing them.
//   3. Apply top-shading by comparing this column's surface height to its
//      northern neighbour — classic "slightly darker if lower, lighter if
//      higher" trick for pseudo-3D relief. Shade multipliers (180/220/255)
//      mirror fastanvil's so the legacy and modern paths blend visually in
//      maps that mix them.

use super::chunk::LegacyChunkData;
use super::chunk_forge112;
use super::palette::LegacyPalette;
use crate::anvil::palette::PaletteFormat;
use crate::anvil::pipeline::{RenderEngine, Rgba};

const SECTION_Y_MAX: i32 = 255;

#[cfg(test)]
mod tests;

/// Pre-1.13 top-shade renderer. Plugs into the shared region pipeline; the
/// format tag selects which NBT decoder to run.
pub struct LegacyTopShadeRenderer<'a> {
    palette: &'a LegacyPalette,
    format: PaletteFormat,
}

impl<'a> LegacyTopShadeRenderer<'a> {
    pub fn new(palette: &'a LegacyPalette, format: PaletteFormat) -> Self {
        Self { palette, format }
    }

    fn render_column(
        &self,
        chunk: &LegacyChunkData,
        x: usize,
        z: usize,
    ) -> (Rgba, i32, Option<i32>) {
        // HeightMap is "Y of first skylit block" — i.e. one above the top
        // opaque block. Subtract one to start at that top block. If the
        // heightmap is absent or obviously bogus, fall back to scanning from
        // the top of the world.
        let start_y = chunk
            .heightmap_at(x, z)
            .map(|h| (h - 1).clamp(0, SECTION_Y_MAX))
            .unwrap_or(SECTION_Y_MAX);

        let mut acc = [0f32; 4];
        let mut surface: Option<i32> = None;
        let mut raw_surface = None;

        let mut y = start_y;
        while y >= 0 {
            let (id, meta) = chunk.get(x, y as usize, z);
            if id == 0 {
                y -= 1;
                continue;
            }
            // Neighbour shading follows nonzero IDs even when their palette colour is transparent.
            raw_surface.get_or_insert(y);
            let color = self.palette.lookup(id, meta);
            if color[3] == 0 {
                y -= 1;
                continue;
            }
            if surface.is_none() {
                surface = Some(y);
            }
            composite_under(&mut acc, color);
            if acc[3] >= 0.999 {
                break;
            }
            y -= 1;
        }

        let rgba = finalize(acc);
        (rgba, surface.unwrap_or(0), raw_surface)
    }
}

pub enum NorthBoundary {
    Heights([Option<i32>; 16]),
    Deferred(Box<LegacyChunkData>),
}

impl<'a> RenderEngine for LegacyTopShadeRenderer<'a> {
    type Chunk = LegacyChunkData;
    type Boundary = NorthBoundary;
    fn boundary(&self, chunk: LegacyChunkData) -> NorthBoundary {
        NorthBoundary::Deferred(Box::new(chunk))
    }
    fn render_chunk(
        &self,
        chunk: LegacyChunkData,
        north: Option<&NorthBoundary>,
    ) -> ([Rgba; 256], NorthBoundary) {
        let mut out = [[0u8; 4]; 16 * 16];
        let mut previous_row = [None; 16];
        for z in 0..16usize {
            for x in 0..16usize {
                let (color, height, raw_surface) = self.render_column(&chunk, x, z);
                let neighbour_height = if z == 0 {
                    // Top row of the chunk uses the south-most row of the
                    // chunk immediately above (to the north in world coords).
                    north
                        .and_then(|n| match n {
                            NorthBoundary::Heights(heights) => heights[x],
                            NorthBoundary::Deferred(chunk) => surface_height(chunk, x, 15),
                        })
                        .unwrap_or(height)
                } else {
                    previous_row[x].unwrap_or(height)
                };
                let shaded = apply_shade(color, height, neighbour_height);
                out[z * 16 + x] = shaded;
                previous_row[x] = raw_surface;
            }
        }
        (out, NorthBoundary::Heights(previous_row))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Option<Self::Chunk>, String> {
        let decoded = match self.format {
            PaletteFormat::Forge112 => chunk_forge112::from_bytes(bytes),
            // Modern shouldn't reach the legacy engine at all (the dispatcher
            // routes elsewhere), but treating it like 1.7.10 here is safe.
            PaletteFormat::Legacy17 | PaletteFormat::Modern => LegacyChunkData::from_bytes(bytes),
        }?;
        Ok(Some(decoded))
    }
}

/// Composite `top` underneath the currently-accumulated color. Standard
/// "src-over" alpha blending in straight-alpha form (palette entries aren't
/// premultiplied).
fn composite_under(acc: &mut [f32; 4], top: Rgba) {
    let a = top[3] as f32 / 255.0;
    let remaining = 1.0 - acc[3];
    let weight = a * remaining;
    acc[0] += top[0] as f32 * weight;
    acc[1] += top[1] as f32 * weight;
    acc[2] += top[2] as f32 * weight;
    acc[3] += weight;
}

fn finalize(acc: [f32; 4]) -> Rgba {
    if acc[3] <= 0.0 {
        return [0, 0, 0, 0];
    }
    let inv = 1.0 / acc[3];
    [
        (acc[0] * inv).round().clamp(0.0, 255.0) as u8,
        (acc[1] * inv).round().clamp(0.0, 255.0) as u8,
        (acc[2] * inv).round().clamp(0.0, 255.0) as u8,
        (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// First nonzero block ID used for northern-neighbour shading.
fn surface_height(chunk: &LegacyChunkData, x: usize, z: usize) -> Option<i32> {
    let start_y = chunk
        .heightmap_at(x, z)
        .map(|h| (h - 1).clamp(0, SECTION_Y_MAX))
        .unwrap_or(SECTION_Y_MAX);
    let mut y = start_y;
    while y >= 0 {
        let (id, _) = chunk.get(x, y as usize, z);
        if id != 0 {
            return Some(y);
        }
        y -= 1;
    }
    None
}

/// Shade multiplier per fastanvil convention: 180 / 220 / 255 for
/// lower / same / higher than neighbour (north).
fn apply_shade(color: Rgba, height: i32, neighbour: i32) -> Rgba {
    if color[3] == 0 {
        return color;
    }
    let mul: u16 = if height > neighbour {
        255
    } else if height == neighbour {
        220
    } else {
        180
    };
    [
        scale(color[0], mul),
        scale(color[1], mul),
        scale(color[2], mul),
        color[3],
    ]
}

#[inline]
fn scale(c: u8, mul: u16) -> u8 {
    ((c as u16 * mul) / 255) as u8
}
