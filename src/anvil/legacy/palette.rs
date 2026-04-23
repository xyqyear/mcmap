// Palette for pre-1.13 worlds (1.7.10 + NEID, or Forge 1.12.2 + REI).
//
// Keyed by numeric block ID and (optionally) metadata. The JSON on disk is
// wrapped with a `"format"` sentinel — `"1.7.10"` or `"1.12.2"` — so the
// renderer can pick the matching chunk decoder.
//
// The lookup shape is the same for both formats: every block decodes to an
// `(id, meta)` pair regardless of what the on-disk chunk encoding looked like.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub type Rgba = [u8; 4];

/// File-on-disk shape.
#[derive(Deserialize, Serialize, Debug)]
pub struct LegacyPaletteFile {
    pub format: String,
    pub blocks: HashMap<String, Rgba>,
}

/// In-memory lookup structure. Keys of the JSON map are flattened into two
/// HashMaps so `lookup(id, meta)` is a couple of O(1) probes with no parsing.
#[derive(Debug, Clone)]
pub struct LegacyPalette {
    by_id_meta: HashMap<(u16, u16), Rgba>,
    by_id: HashMap<u16, Rgba>,
    /// Used when neither `id|meta` nor `id` has an entry. Fully transparent
    /// so unknown blocks fall through to whatever is below them (matching
    /// the "treat as air" convention of the renderer).
    default: Rgba,
}

impl LegacyPalette {
    /// Parse from a `LegacyPaletteFile`. Keys must be `"<id>"` or
    /// `"<id>|<meta>"`; anything else is skipped with a warning via the
    /// supplied logger callback (keeps us from pulling `log` into this leaf
    /// module just to warn on malformed entries).
    pub fn from_file(file: LegacyPaletteFile) -> Self {
        let mut by_id_meta = HashMap::with_capacity(file.blocks.len());
        let mut by_id = HashMap::new();
        for (key, color) in file.blocks {
            match parse_key(&key) {
                Some((id, Some(meta))) => {
                    by_id_meta.insert((id, meta), color);
                }
                Some((id, None)) => {
                    by_id.insert(id, color);
                }
                None => {
                    log::debug!("Skipping malformed legacy palette key: {}", key);
                }
            }
        }
        Self {
            by_id_meta,
            by_id,
            default: [0, 0, 0, 0],
        }
    }

    /// Load + parse a `.json` palette from disk. Accepts both legacy formats
    /// (1.7.10 vanilla/NEID and 1.12.2 REI) — the in-memory shape is the same;
    /// the format string only matters at the renderer level for chunk-decoder
    /// dispatch.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let file: LegacyPaletteFile = serde_json::from_slice(&bytes)?;
        match file.format.as_str() {
            "1.7.10" | "1.12.2" => Ok(Self::from_file(file)),
            other => Err(format!(
                "Expected legacy palette with format='1.7.10' or '1.12.2', got '{}'",
                other
            )
            .into()),
        }
    }

    /// Most specific match wins: `id|meta` → `id` → transparent default.
    #[inline]
    pub fn lookup(&self, id: u16, meta: u16) -> Rgba {
        if let Some(&c) = self.by_id_meta.get(&(id, meta)) {
            return c;
        }
        if let Some(&c) = self.by_id.get(&id) {
            return c;
        }
        self.default
    }

    /// Count of distinct (id, meta) + id entries. For logging only.
    pub fn len(&self) -> usize {
        self.by_id_meta.len() + self.by_id.len()
    }
}

fn parse_key(k: &str) -> Option<(u16, Option<u16>)> {
    if let Some((id_s, meta_s)) = k.split_once('|') {
        let id: u16 = id_s.parse().ok()?;
        let meta: u16 = meta_s.parse().ok()?;
        Some((id, Some(meta)))
    } else {
        let id: u16 = k.parse().ok()?;
        Some((id, None))
    }
}

/// Peek at a palette JSON file to determine its format without fully parsing
/// it. Looks for the `"format"` field at the root; if absent (flat map of
/// `"namespace:name" -> rgba`) we assume modern 1.13+.
pub fn detect_palette_format(
    path: &Path,
) -> Result<PaletteFormat, Box<dyn std::error::Error>> {
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

/// Which on-disk format a palette JSON came from. The renderer uses this to
/// pick the chunk decoder; the in-memory `LegacyPalette` is identical for
/// the two legacy variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFormat {
    Modern,
    /// 1.7.10 vanilla, optionally with NotEnoughIDs (Blocks16/Data16).
    Legacy17,
    /// Forge 1.12.2 with RoughlyEnoughIDs / JustEnoughIDs (per-section
    /// Palette IntArray).
    Forge112,
}
