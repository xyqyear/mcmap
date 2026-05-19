use std::fs;
use std::path::{Path, PathBuf};

use super::output::{PlayerIdKind, StorageKind};

#[derive(Debug, Clone)]
pub struct PlayerFile {
    pub path: PathBuf,
    pub source: String,
    pub storage: StorageKind,
    pub id: String,
    pub id_kind: PlayerIdKind,
}

pub fn discover(world_dir: &Path) -> Result<Vec<PlayerFile>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    collect_from_dir(
        world_dir,
        &world_dir.join("players").join("data"),
        StorageKind::PlayersData,
        &mut files,
    )?;
    collect_from_dir(
        world_dir,
        &world_dir.join("playerdata"),
        StorageKind::Playerdata,
        &mut files,
    )?;
    collect_from_dir(
        world_dir,
        &world_dir.join("players"),
        StorageKind::LegacyPlayers,
        &mut files,
    )?;
    files.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(files)
}

fn collect_from_dir(
    world_dir: &Path,
    dir: &Path,
    storage: StorageKind,
    out: &mut Vec<PlayerFile>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("dat") {
            continue;
        }
        let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let source = relative_path(world_dir, &path);
        out.push(PlayerFile {
            path,
            source,
            storage,
            id_kind: classify_id(&stem),
            id: stem,
        });
    }
    Ok(())
}

fn classify_id(stem: &str) -> PlayerIdKind {
    if is_dashed_uuid(stem) {
        PlayerIdKind::Uuid
    } else {
        PlayerIdKind::Name
    }
}

fn is_dashed_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, b) in bytes.iter().copied().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

pub fn relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_dashed_uuid() {
        assert_eq!(
            classify_id("0b4c4192-8eb3-4f0b-9022-8e2cb2ee6fc0"),
            PlayerIdKind::Uuid
        );
        assert_eq!(
            classify_id("0b4c41928eb34f0b90228e2cb2ee6fc0"),
            PlayerIdKind::Name
        );
        assert_eq!(classify_id("notch"), PlayerIdKind::Name);
    }
}
