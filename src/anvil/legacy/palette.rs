// Palette for pre-1.13 worlds (1.7.10 + NEID, or Forge 1.12.2 + REI).
//
// Keyed by numeric block ID and (optionally) metadata. The JSON on disk is
// wrapped with a `"format"` sentinel — `"1.7.10"` or `"1.12.2"` — read by
// `anvil::palette::detect_format` to pick the matching chunk decoder. The
// in-memory shape is identical for both formats.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

pub type Rgba = [u8; 4];

/// File-on-disk shape.
#[derive(Deserialize, Serialize, Debug)]
pub struct LegacyPaletteFile {
    pub format: String,
    pub blocks: HashMap<String, Rgba>,
}

/// Numeric IDs index compact records; ordinary metadata has direct slots.
#[derive(Debug, Clone)]
pub struct LegacyPalette {
    by_id: Box<[u32]>,
    entries: Vec<IdColours>,
    extended_meta: HashMap<(u16, u16), Rgba>,
    len: usize,
    /// Used when neither `id|meta` nor `id` has an entry. Fully transparent
    /// so unknown blocks fall through to whatever is below them (matching
    /// the "treat as air" convention of the renderer).
    default: Rgba,
}

#[derive(Debug, Clone, Default)]
struct IdColours {
    fallback: Option<Rgba>,
    variants: Option<Box<[Option<Rgba>; 16]>>,
}

impl LegacyPalette {
    /// Parse from a `LegacyPaletteFile`. Keys must be `"<id>"` or
    /// `"<id>|<meta>"`; anything else is skipped (logged at debug).
    pub fn from_file(file: LegacyPaletteFile) -> Self {
        let slots = file
            .blocks
            .keys()
            .filter_map(|key| parse_key(key))
            .map(|(id, _)| usize::from(id) + 1)
            .max()
            .unwrap_or(0);
        let mut by_id = vec![0_u32; slots].into_boxed_slice();
        let mut entries: Vec<IdColours> = Vec::new();
        let mut extended_meta = HashMap::new();
        let mut len = 0;
        for (key, color) in file.blocks {
            match parse_key(&key) {
                Some((id, meta)) => {
                    let slot = &mut by_id[usize::from(id)];
                    if *slot == 0 {
                        entries.push(IdColours::default());
                        *slot = entries.len() as u32;
                    }
                    let entry = &mut entries[*slot as usize - 1];
                    let previous = match meta {
                        None => entry.fallback.replace(color),
                        Some(meta @ 0..=15) => entry
                            .variants
                            .get_or_insert_with(|| Box::new([None; 16]))[usize::from(meta)]
                        .replace(color),
                        Some(meta) => extended_meta.insert((id, meta), color),
                    };
                    len += usize::from(previous.is_none());
                }
                None => {
                    log::debug!("Skipping malformed legacy palette key: {}", key);
                }
            }
        }
        Self {
            by_id,
            entries,
            extended_meta,
            len,
            default: [0, 0, 0, 0],
        }
    }

    /// Parse a `.json` palette. Accepts both legacy formats
    /// (1.7.10 vanilla/NEID and 1.12.2 REI) — the in-memory shape is the same;
    /// the format string is read by `anvil::palette::detect_format` at the
    /// renderer level for chunk-decoder dispatch.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let file: LegacyPaletteFile = serde_json::from_slice(bytes)?;
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
        if meta >= 16
            && let Some(&color) = self.extended_meta.get(&(id, meta))
        {
            return color;
        }
        let slot = self.by_id.get(usize::from(id)).copied().unwrap_or(0);
        if slot == 0 {
            return self.default;
        }
        let entry = &self.entries[slot as usize - 1];
        let exact = if meta < 16 {
            entry
                .variants
                .as_ref()
                .and_then(|variants| variants[usize::from(meta)])
        } else {
            None
        };
        exact.or(entry.fallback).unwrap_or(self.default)
    }

    /// Count of distinct (id, meta) + id entries. For logging only.
    pub fn len(&self) -> usize {
        self.len
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
