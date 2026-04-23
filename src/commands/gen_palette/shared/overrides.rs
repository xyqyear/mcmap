use fastanvil::Rgba;
use std::collections::HashMap;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Parse a user overrides file. Format: `{"<key>": [r,g,b,a], ...}` — keyed by
/// whatever the per-version palette uses (`"namespace:id"` / `"id"` /
/// `"id|meta"`), which is opaque to the loader.
pub fn load_overrides(path: &Path) -> Result<HashMap<String, Rgba>> {
    let bytes = std::fs::read(path)?;
    let map: HashMap<String, Rgba> = serde_json::from_slice(&bytes)?;
    Ok(map)
}

/// Apply every entry in `overrides` onto `palette`, replacing any existing
/// color under the same key. Returns the number of entries applied.
pub fn apply_overrides(
    palette: &mut HashMap<String, Rgba>,
    overrides: HashMap<String, Rgba>,
) -> usize {
    let n = overrides.len();
    for (k, v) in overrides {
        palette.insert(k, v);
    }
    n
}
