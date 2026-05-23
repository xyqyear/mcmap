// Dimension-id -> on-disk dimension folder resolution shared by extractors.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct DimensionEntry {
    /// Raw dimension id. ResourceLocation string for modern worlds
    /// (`"minecraft:overworld"`); decimal int string for legacy worlds
    /// (`"0"`, `"-1"`, `"7"`).
    pub id: String,
    /// Path relative to `world_dir`. `"."` for the overworld.
    pub folder: String,
    /// Whether `<world_dir>/<folder>/region/` exists on disk.
    pub exists: bool,
}

/// ResourceLocation -> folder. Vanilla dimensions probe the pre-26.1 layout
/// first, then the 26.1+ `dimensions/minecraft/...` layout.
pub fn resolve_modern(world_dir: &Path, dim_id: &str) -> PathBuf {
    match dim_id {
        "minecraft:overworld" => resolve_first_existing_region([
            world_dir.to_path_buf(),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("overworld"),
        ]),
        "minecraft:the_nether" => resolve_first_existing_region([
            world_dir.join("DIM-1"),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("the_nether"),
        ]),
        "minecraft:the_end" => resolve_first_existing_region([
            world_dir.join("DIM1"),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("the_end"),
        ]),
        s => {
            let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
            world_dir.join("dimensions").join(ns).join(path)
        }
    }
}

fn resolve_first_existing_region<const N: usize>(candidates: [PathBuf; N]) -> PathBuf {
    let mut iter = candidates.into_iter();
    let default = iter
        .next()
        .expect("dimension path candidates must not be empty");
    if default.join("region").is_dir() {
        return default;
    }
    iter.find(|path| path.join("region").is_dir())
        .unwrap_or(default)
}

/// Pre-1.13 int dim id -> (folder, exists). Tries `DIM<N>/region/` first, then
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
                if dim_id >= 0 && prefix.ends_with('-') {
                    continue;
                }
                if !prefix.is_empty() && !prefix.chars().last().unwrap().is_ascii_digit() {
                    return (entry.path(), true);
                }
            }
        }
    }
    (default, false)
}

/// Convert an absolute folder path into a JSON-friendly path relative to
/// `world_dir`. Overworld -> `"."`. Always uses `/` separator.
pub fn folder_to_relative(world_dir: &Path, folder: &Path) -> String {
    if folder == world_dir {
        return ".".into();
    }
    folder
        .strip_prefix(world_dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| folder.to_string_lossy().to_string())
}

pub fn entry_for_modern(world_dir: &Path, dim_id: &str) -> DimensionEntry {
    let folder = resolve_modern(world_dir, dim_id);
    DimensionEntry {
        id: dim_id.to_string(),
        folder: folder_to_relative(world_dir, &folder),
        exists: folder.join("region").is_dir(),
    }
}

pub fn entry_for_legacy(world_dir: &Path, dim_id: i32) -> DimensionEntry {
    let (folder, exists) = resolve_legacy(world_dir, dim_id);
    DimensionEntry {
        id: dim_id.to_string(),
        folder: folder_to_relative(world_dir, &folder),
        exists,
    }
}

pub fn entry_for_id(world_dir: &Path, dim_id: &str) -> DimensionEntry {
    match dim_id.parse::<i32>() {
        Ok(dim) => entry_for_legacy(world_dir, dim),
        Err(_) => entry_for_modern(world_dir, dim_id),
    }
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
    fn modern_vanilla_dims_fallback_to_namespaced_layout() {
        let w = tmpdir();
        fs::create_dir_all(w.join("dimensions/minecraft/overworld/region")).unwrap();
        fs::create_dir_all(w.join("dimensions/minecraft/the_nether/region")).unwrap();
        fs::create_dir_all(w.join("dimensions/minecraft/the_end/region")).unwrap();

        assert_eq!(
            resolve_modern(&w, "minecraft:overworld"),
            w.join("dimensions/minecraft/overworld")
        );
        assert_eq!(
            resolve_modern(&w, "minecraft:the_nether"),
            w.join("dimensions/minecraft/the_nether")
        );
        assert_eq!(
            resolve_modern(&w, "minecraft:the_end"),
            w.join("dimensions/minecraft/the_end")
        );

        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn modern_vanilla_dims_prefer_old_layout() {
        let w = tmpdir();
        fs::create_dir_all(w.join("region")).unwrap();
        fs::create_dir_all(w.join("DIM-1/region")).unwrap();
        fs::create_dir_all(w.join("DIM1/region")).unwrap();
        fs::create_dir_all(w.join("dimensions/minecraft/overworld/region")).unwrap();
        fs::create_dir_all(w.join("dimensions/minecraft/the_nether/region")).unwrap();
        fs::create_dir_all(w.join("dimensions/minecraft/the_end/region")).unwrap();

        assert_eq!(resolve_modern(&w, "minecraft:overworld"), w);
        assert_eq!(resolve_modern(&w, "minecraft:the_nether"), w.join("DIM-1"));
        assert_eq!(resolve_modern(&w, "minecraft:the_end"), w.join("DIM1"));

        let _ = fs::remove_dir_all(&w);
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
        let w = tmpdir();
        fs::create_dir_all(w.join("DIM110/region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 10);
        assert_eq!(folder, w.join("DIM10"));
        assert!(!exists);
        let _ = fs::remove_dir_all(&w);
    }

    #[test]
    fn legacy_positive_dim_does_not_match_negative_folder() {
        let w = tmpdir();
        fs::create_dir_all(w.join("DIM-1/region")).unwrap();
        let (folder, exists) = resolve_legacy(&w, 1);
        assert_eq!(folder, w.join("DIM1"));
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
