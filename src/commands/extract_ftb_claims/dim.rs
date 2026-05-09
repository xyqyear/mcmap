// Dim-ID → on-disk dim folder resolution.
//
// 1.16+ (SNBT family): rule is exact and final. The three vanilla dims sit at
// `world/region/`, `world/DIM-1/region/`, `world/DIM1/region/`; everything
// else is `world/dimensions/<ns>/<path>/region/`. Mods cannot override.
//
// Pre-1.13 (A0/A1/A2): default is `DIM<N>/`, but mods like Galacticraft,
// Dimensional Doors, Underground Biomes, Mystcraft override to
// `DIM_SPACESTATION<N>`, `PERSONAL_DIM_<N>`, etc. The same numeric id can map
// to different folders across worlds, so we probe disk: try the default
// first, then scan for any subdir whose name ends in `<N>` (preceded by a
// non-digit character) that contains `region/`.

use std::fs;
use std::path::{Path, PathBuf};

/// 1.16+ ResourceLocation → folder. Returns the path; the caller computes
/// `exists` separately so the output schema can flag missing folders.
pub fn resolve_modern(world_dir: &Path, dim_id: &str) -> PathBuf {
    match dim_id {
        "minecraft:overworld" => world_dir.to_path_buf(),
        "minecraft:the_nether" => world_dir.join("DIM-1"),
        "minecraft:the_end" => world_dir.join("DIM1"),
        s => {
            let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
            world_dir.join("dimensions").join(ns).join(path)
        }
    }
}

/// Pre-1.13 int dim id → (folder, exists). Tries `DIM<N>/region/` first, then
/// scans `world_dir` for any sibling matching `<prefix><N>` where `<prefix>`
/// ends in a non-digit character and contains `region/`.
pub fn resolve_legacy(world_dir: &Path, dim_id: i32) -> (PathBuf, bool) {
    if dim_id == 0 {
        let exists = world_dir.join("region").is_dir();
        return (world_dir.to_path_buf(), exists);
    }
    let default = world_dir.join(format!("DIM{}", dim_id));
    if default.join("region").is_dir() {
        return (default, true);
    }
    let target = dim_id.to_string();
    if let Ok(entries) = fs::read_dir(world_dir) {
        for entry in entries.flatten() {
            if !entry.path().join("region").is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(prefix) = name_str.strip_suffix(&*target) {
                if !prefix.is_empty()
                    && !prefix.chars().last().unwrap().is_ascii_digit()
                {
                    return (entry.path(), true);
                }
            }
        }
    }
    (default, false)
}

/// Convert an absolute folder path into a JSON-friendly path relative to
/// `world_dir`. Overworld → `"."`. Always uses `/` separator.
pub fn folder_to_relative(world_dir: &Path, folder: &Path) -> String {
    if folder == world_dir {
        return ".".into();
    }
    folder
        .strip_prefix(world_dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| folder.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let id = format!(
            "mcmap_dim_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let p = std::env::temp_dir().join(id);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn modern_vanilla_dims() {
        let w = PathBuf::from("/world");
        assert_eq!(resolve_modern(&w, "minecraft:overworld"), w);
        assert_eq!(resolve_modern(&w, "minecraft:the_nether"), w.join("DIM-1"));
        assert_eq!(resolve_modern(&w, "minecraft:the_end"), w.join("DIM1"));
    }

    #[test]
    fn modern_modded_dims() {
        let w = PathBuf::from("/world");
        assert_eq!(
            resolve_modern(&w, "allthemodium:mining"),
            w.join("dimensions/allthemodium/mining")
        );
    }

    #[test]
    fn legacy_default_path() {
        let w = tmpdir();
        fs::create_dir_all(w.join("DIM7/region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 7);
        assert_eq!(folder, w.join("DIM7"));
        assert!(exists);
        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn legacy_galacticraft_override() {
        let w = tmpdir();
        fs::create_dir_all(w.join("DIM_MOTHERSHIP11/region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 11);
        assert_eq!(folder, w.join("DIM_MOTHERSHIP11"));
        assert!(exists);
        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn legacy_no_match_returns_default_with_false() {
        let w = tmpdir();
        let (folder, exists) = resolve_legacy(&w, 999);
        assert_eq!(folder, w.join("DIM999"));
        assert!(!exists);
        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn legacy_avoids_digit_collision() {
        // DIM110 must NOT match dim id 10 (suffix "10" is preceded by '1', a digit).
        let w = tmpdir();
        fs::create_dir_all(w.join("DIM110/region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 10);
        assert_eq!(folder, w.join("DIM10"));
        assert!(!exists);
        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn legacy_overworld_dim_zero() {
        let w = tmpdir();
        fs::create_dir_all(w.join("region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 0);
        assert_eq!(folder, w);
        assert!(exists);
        let _ = fs::remove_dir_all(&w);
    }
}
