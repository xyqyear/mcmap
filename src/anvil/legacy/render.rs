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
//      higher" trick for pseudo-3D relief.

use super::chunk::LegacyChunkData;
use super::chunk_forge112;
use super::palette::{LegacyPalette, PaletteFormat, Rgba};
use crate::anvil::region::{CCoord, RCoord, RegionLoader};
use crate::anvil::render::RegionMap;

const SECTION_Y_MAX: i32 = 255;

/// Render-time state. `shade` mirrors the shade factors used by
/// `fastanvil::TopShadeRenderer` so the legacy path blends visually with the
/// modern path in maps that mix them.
pub struct LegacyTopShadeRenderer<'a> {
    palette: &'a LegacyPalette,
}

impl<'a> LegacyTopShadeRenderer<'a> {
    pub fn new(palette: &'a LegacyPalette) -> Self {
        Self { palette }
    }

    /// Render a single chunk into a 16×16 block of Rgba pixels, YZX row-major
    /// (z outer, x inner — matches `RegionMap::chunk`).
    pub fn render(
        &self,
        chunk: &LegacyChunkData,
        north: Option<&LegacyChunkData>,
    ) -> [Rgba; 16 * 16] {
        let mut out = [[0u8; 4]; 16 * 16];
        for z in 0..16usize {
            for x in 0..16usize {
                let (color, height) = self.render_column(chunk, x, z);
                let neighbour_height = if z == 0 {
                    // The top row of the chunk uses the south-most row of the
                    // chunk immediately above (to the north in world coords).
                    north
                        .and_then(|n| surface_height(n, x, 15))
                        .unwrap_or(height)
                } else {
                    surface_height(chunk, x, z - 1).unwrap_or(height)
                };
                let shaded = apply_shade(color, height, neighbour_height);
                out[z * 16 + x] = shaded;
            }
        }
        out
    }

    fn render_column(
        &self,
        chunk: &LegacyChunkData,
        x: usize,
        z: usize,
    ) -> (Rgba, i32) {
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

        let mut y = start_y;
        while y >= 0 {
            let (id, meta) = chunk.get(x, y as usize, z);
            if id == 0 {
                y -= 1;
                continue;
            }
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
        (rgba, surface.unwrap_or(0))
    }
}

/// Composite `top` underneath the currently-accumulated color. Standard
/// "src-over" alpha blending in premultiplied terms, but we use straight
/// alpha throughout for simplicity (palette entries aren't premultiplied).
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

/// Find the Y of the top opaque block for shading. Cheaper than a full
/// composite — we only need a single number.
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

/// Decode a legacy chunk's NBT bytes using the parser appropriate for the
/// active palette format. Returns `Err` only on a real parse failure — chunks
/// with wrong-format payloads end up as `Err` at the type-mismatch level.
fn decode_chunk(bytes: &[u8], format: PaletteFormat) -> Result<LegacyChunkData, String> {
    match format {
        PaletteFormat::Forge112 => chunk_forge112::from_bytes(bytes),
        // Modern shouldn't reach the legacy renderer at all (the dispatcher
        // routes elsewhere), but treating it like 1.7.10 here is safe.
        PaletteFormat::Legacy17 | PaletteFormat::Modern => LegacyChunkData::from_bytes(bytes),
    }
}

/// Region-level driver for the legacy renderer — mirrors the modern
/// `render_region` function but operates on `LegacyChunkData`. The
/// `chunk_format` selects the parser.
pub fn render_legacy_region(
    x: RCoord,
    z: RCoord,
    loader: &dyn RegionLoader,
    renderer: LegacyTopShadeRenderer,
    chunk_format: PaletteFormat,
) -> Result<Option<RegionMap>, String> {
    let mut map = RegionMap::new(x, z, [0u8; 4]);

    let mut region = match loader.region(x, z)? {
        Some(r) => r,
        None => return Ok(None),
    };

    // Cache the last row of chunks from the region immediately above for
    // top-shading continuity across region boundaries.
    let mut cache: [Option<LegacyChunkData>; 32] = Default::default();
    if let Ok(Some(mut r)) = loader.region(x, RCoord(z.0 - 1)) {
        for (cx, entry) in cache.iter_mut().enumerate() {
            *entry = r
                .read_chunk(cx, 31)
                .ok()
                .flatten()
                .and_then(|b| decode_chunk(&b, chunk_format).ok());
        }
    }

    for cz in 0usize..32 {
        for (cx, cache) in cache.iter_mut().enumerate() {
            let data = map.chunk_mut_by_coord(CCoord(cx as isize), CCoord(cz as isize));

            let chunk_bytes = region
                .read_chunk(cx, cz)
                .map_err(|e| format!("Failed to read chunk: {}", e))?;
            let chunk_bytes = match chunk_bytes {
                Some(b) => b,
                None => continue,
            };

            let chunk = match decode_chunk(&chunk_bytes, chunk_format) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Skipping malformed legacy chunk ({},{}): {}", cx, cz, e);
                    continue;
                }
            };

            let rendered = renderer.render(&chunk, cache.as_ref());
            data.copy_from_slice(&rendered);
            *cache = Some(chunk);
        }
    }

    Ok(Some(map))
}
