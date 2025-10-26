// Region file access - using fastanvil's Region but with custom chunk parsing

use std::fs::File;
use std::path::{Path, PathBuf};

pub use fastanvil::{CCoord, RCoord};

/// Trait for loading regions
pub trait RegionLoader {
    fn region(&self, x: RCoord, z: RCoord) -> Result<Option<Region>, String>;
}

/// Region file wrapper
pub struct Region {
    inner: fastanvil::Region<File>,
}

impl Region {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let inner = fastanvil::Region::from_stream(file).map_err(|e| e.to_string())?;
        Ok(Self { inner })
    }

    pub fn read_chunk(&mut self, x: usize, z: usize) -> Result<Option<Vec<u8>>, String> {
        self.inner.read_chunk(x, z).map_err(|e| e.to_string())
    }
}

/// Region file loader
pub struct RegionFileLoader {
    root: PathBuf,
}

impl RegionFileLoader {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn list(&self) -> Result<Vec<(RCoord, RCoord)>, String> {
        let mut coords = Vec::new();
        let entries = std::fs::read_dir(&self.root).map_err(|e| e.to_string())?;

        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if let Some((x, z)) = parse_region_filename(&filename_str) {
                coords.push((RCoord(x), RCoord(z)));
            }
        }

        Ok(coords)
    }
}

impl RegionLoader for RegionFileLoader {
    fn region(&self, x: RCoord, z: RCoord) -> Result<Option<Region>, String> {
        let path = self.root.join(format!("r.{}.{}.mca", x.0, z.0));
        if !path.exists() {
            return Ok(None);
        }
        Region::from_file(path).map(Some)
    }
}

fn parse_region_filename(filename: &str) -> Option<(isize, isize)> {
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        return None;
    }

    let x: isize = parts[1].parse().ok()?;
    let z: isize = parts[2].parse().ok()?;
    Some((x, z))
}
