// Unified palette-write seam.
//
// Both palette schemas land here so each pipeline ends with one line instead
// of three near-identical `to_vec_pretty + write + chown` blocks. The two
// shapes stay distinct because the renderer dispatches on them via
// `anvil::palette::detect_format`.

use std::collections::HashMap;
use std::path::Path;

use crate::anvil::legacy::palette::LegacyPaletteFile;
use crate::anvil::palette::Rgba;
use crate::chown;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub enum PaletteOutput<'a> {
    /// Flat `{"namespace:name": [r,g,b,a]}` — 1.13+ pipeline.
    Modern(&'a HashMap<String, Rgba>),
    /// Wrapped `{"format": "...", "blocks": {...}}` — 1.7.10 / Forge 1.12.2.
    Legacy(&'a LegacyPaletteFile),
}

impl PaletteOutput<'_> {
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let bytes = match self {
            Self::Modern(m) => serde_json::to_vec_pretty(m)?,
            Self::Legacy(f) => serde_json::to_vec_pretty(f)?,
        };
        std::fs::write(path, &bytes)?;
        chown::apply(path)
            .map_err(|e| format!("Failed to chown {}: {}", path.display(), e))?;
        Ok(())
    }

    pub fn entry_count(&self) -> usize {
        match self {
            Self::Modern(m) => m.len(),
            Self::Legacy(f) => f.blocks.len(),
        }
    }
}
