// Top-level palette dispatch.
//
// Two on-disk palette formats coexist:
//
// - Modern (1.13+): flat `{"namespace:name": [r,g,b,a]}` — loaded via
//   `modern::RenderedPalette`.
// - Legacy (1.7.10 vanilla/NEID, Forge 1.12.2 REI): wrapped
//   `{"format":"1.7.10"|"1.12.2", "blocks":{"id"|"id|meta": [...]}}` — loaded
//   via `legacy::LegacyPalette`, with the format tag deciding which chunk
//   decoder the renderer plugs in.
//
// `detect_format` peeks at the JSON's root `format` field to route. Absence
// of that field ⇒ modern (the flat map has no such key).

use log::info;
use serde::Deserialize;
use std::path::Path;

use super::legacy::LegacyPalette;
use super::modern::RenderedPalette;

pub type Rgba = [u8; 4];

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Which on-disk palette format was detected. Selects the chunk decoder at
/// render time; the in-memory `LegacyPalette` is identical between the two
/// legacy variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFormat {
    Modern,
    /// 1.7.10 vanilla, optionally with NotEnoughIDs (Blocks16/Data16).
    Legacy17,
    /// Forge 1.12.2 with RoughlyEnoughIDs / JustEnoughIDs (per-section
    /// Palette IntArray).
    Forge112,
}

/// Loaded palette. Carries just enough type info to pick the right chunk
/// decoder at the engine-construction site.
pub enum AnyPalette {
    Modern(RenderedPalette),
    Legacy(LegacyPalette, PaletteFormat),
}

/// Peek at a palette JSON to determine its format without fully parsing it.
pub fn detect_format(path: &Path) -> Result<PaletteFormat> {
    let bytes = std::fs::read(path)?;
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        format: Option<String>,
    }
    let probe: Probe = serde_json::from_slice(&bytes)?;
    match probe.format.as_deref() {
        Some("1.7.10") => Ok(PaletteFormat::Legacy17),
        Some("1.12.2") => Ok(PaletteFormat::Forge112),
        _ => Ok(PaletteFormat::Modern),
    }
}

/// Load a palette from disk, dispatching on its declared format.
pub fn load(path: &Path) -> Result<AnyPalette> {
    info!("Loading palette from: {}", path.display());
    let format = detect_format(path)?;
    match format {
        PaletteFormat::Modern => {
            let bytes = std::fs::read(path)?;
            let blockstates: std::collections::HashMap<String, Rgba> =
                serde_json::from_slice(&bytes)?;
            info!(
                "Palette loaded: {} block states (modern / 1.13+)",
                blockstates.len()
            );
            Ok(AnyPalette::Modern(RenderedPalette::new(blockstates)))
        }
        PaletteFormat::Legacy17 => {
            let pal = LegacyPalette::load(path)?;
            info!("Palette loaded: {} entries (legacy / 1.7.10)", pal.len());
            Ok(AnyPalette::Legacy(pal, format))
        }
        PaletteFormat::Forge112 => {
            let pal = LegacyPalette::load(path)?;
            info!(
                "Palette loaded: {} entries (legacy / Forge 1.12.2 + REI)",
                pal.len()
            );
            Ok(AnyPalette::Legacy(pal, format))
        }
    }
}
