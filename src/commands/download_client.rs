// Download the Minecraft client jar for a given version. Looks up the URL
// via Mojang's official launcher metadata (piston-meta), streams the jar to
// a tmp file while hashing, verifies sha1 + size, then moves it into place.

use clap::Args;
use log::{info, warn};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Args, Debug)]
pub struct DownloadClientArgs {
    /// Minecraft version id (e.g. `1.21.4`). Also accepts the aliases
    /// `latest` and `latest-snapshot`.
    version: String,

    /// Output file path for the client jar. Parent directory must exist.
    target: PathBuf,
}

#[derive(Deserialize)]
struct VersionManifest {
    latest: Latest,
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize)]
struct Latest {
    release: String,
    snapshot: String,
}

#[derive(Deserialize)]
struct VersionEntry {
    id: String,
    url: String,
}

#[derive(Deserialize)]
struct VersionJson {
    downloads: VersionDownloads,
}

#[derive(Deserialize)]
struct VersionDownloads {
    client: DownloadInfo,
}

#[derive(Deserialize)]
struct DownloadInfo {
    sha1: String,
    size: u64,
    url: String,
}

pub fn execute(args: DownloadClientArgs) -> Result<()> {
    let http = reqwest::blocking::Client::builder()
        .user_agent(concat!("mcmap/", env!("CARGO_PKG_VERSION")))
        .build()?;

    info!("Fetching version manifest");
    let manifest: VersionManifest = http
        .get(VERSION_MANIFEST_URL)
        .send()?
        .error_for_status()?
        .json()?;

    let requested_id = match args.version.as_str() {
        "latest" => {
            info!("Resolving 'latest' -> {}", manifest.latest.release);
            manifest.latest.release.clone()
        }
        "latest-snapshot" => {
            info!(
                "Resolving 'latest-snapshot' -> {}",
                manifest.latest.snapshot
            );
            manifest.latest.snapshot.clone()
        }
        v => v.to_string(),
    };

    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == requested_id)
        .ok_or_else(|| format!("Version '{}' not found in manifest", requested_id))?;

    info!("Fetching per-version metadata for {}", entry.id);
    let version_json: VersionJson = http
        .get(&entry.url)
        .send()?
        .error_for_status()?
        .json()?;

    let meta = &version_json.downloads.client;
    info!(
        "Client jar: {} bytes, sha1 {}",
        meta.size, meta.sha1
    );

    let tmp_path = std::env::temp_dir().join(format!("mcmap-client-{}.jar.part", entry.id));

    match hash_existing(&tmp_path)? {
        Some(existing) if existing == meta.sha1 => {
            info!(
                "Reusing cached tmp file at {} (sha1 matches)",
                tmp_path.display()
            );
        }
        Some(_) => {
            info!(
                "Cached tmp file at {} has wrong sha1; re-downloading",
                tmp_path.display()
            );
            fetch_to_tmp(&http, meta, &tmp_path)?;
        }
        None => {
            info!("Downloading to {}", tmp_path.display());
            fetch_to_tmp(&http, meta, &tmp_path)?;
        }
    }

    info!("Verification passed. Moving to {}", args.target.display());
    move_or_copy(&tmp_path, &args.target)?;

    info!(
        "Saved client.jar for {} to {}",
        entry.id,
        args.target.display()
    );
    Ok(())
}

fn fetch_to_tmp(
    http: &reqwest::blocking::Client,
    meta: &DownloadInfo,
    tmp_path: &Path,
) -> Result<()> {
    let result = download_and_verify(http, meta, tmp_path);
    if result.is_err() {
        let _ = std::fs::remove_file(tmp_path);
    }
    result
}

fn download_and_verify(
    http: &reqwest::blocking::Client,
    meta: &DownloadInfo,
    tmp_path: &Path,
) -> Result<()> {
    let mut response = http.get(&meta.url).send()?.error_for_status()?;
    let mut file = File::create(tmp_path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = response.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        total += n as u64;
    }
    file.sync_all()?;

    if total != meta.size {
        return Err(format!(
            "Downloaded size mismatch: expected {} bytes, got {}",
            meta.size, total
        )
        .into());
    }

    let sha1_hex = hex_digest(hasher.finalize().as_slice());
    if sha1_hex != meta.sha1 {
        return Err(format!(
            "Downloaded sha1 mismatch: expected {}, got {}",
            meta.sha1, sha1_hex
        )
        .into());
    }
    Ok(())
}

fn hash_existing(path: &Path) -> Result<Option<String>> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(hex_digest(hasher.finalize().as_slice())))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

fn move_or_copy(src: &Path, dst: &Path) -> Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!(
                "Atomic rename failed ({}); falling back to copy + remove",
                e
            );
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
    }
}
