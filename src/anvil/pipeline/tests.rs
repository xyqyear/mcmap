use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{CCoord, RCoord, RenderEngine, Rgba, render_region};
use crate::anvil::region::RegionFileLoader;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcmap-boundary-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn region(&self, z: i32, chunks: &[(usize, usize, u8)]) {
        let mut region = fastanvil::Region::create(Cursor::new(Vec::new())).unwrap();
        for &(x, z, id) in chunks {
            region.write_chunk(x, z, &[id]).unwrap();
        }
        let bytes = region.into_inner().unwrap().into_inner();
        std::fs::write(self.0.join(format!("r.0.{z}.mca")), bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Engine;
impl RenderEngine for Engine {
    type Chunk = u8;
    type Boundary = u8;
    fn decode(&self, bytes: &[u8]) -> Result<Option<u8>, String> {
        match bytes[0] {
            0 => Ok(None),
            255 => Err("invalid".into()),
            id => Ok(Some(id)),
        }
    }
    fn boundary(&self, chunk: u8) -> u8 {
        chunk
    }
    fn render_chunk(&self, chunk: u8, north: Option<&u8>) -> ([Rgba; 256], u8) {
        ([[north.copied().unwrap_or(0), chunk, 0, 255]; 256], chunk)
    }
}

#[test]
fn missing_and_skipped_chunks_retain_the_last_northern_boundary() {
    let fixture = Fixture::new();
    fixture.region(-1, &[(0, 31, 8), (1, 31, 255)]);
    fixture.region(0, &[(0, 1, 2), (0, 2, 0), (0, 3, 3), (1, 0, 7)]);
    let loader = RegionFileLoader::new(fixture.0.clone());
    let map = render_region(RCoord(0), RCoord(0), &loader, &Engine)
        .unwrap()
        .unwrap();
    assert_eq!(map.chunk(CCoord(0), CCoord(0)), &[[0; 4]; 256]);
    assert_eq!(map.chunk(CCoord(0), CCoord(1)), &[[8, 2, 0, 255]; 256]);
    assert_eq!(map.chunk(CCoord(0), CCoord(2)), &[[0; 4]; 256]);
    assert_eq!(map.chunk(CCoord(0), CCoord(3)), &[[2, 3, 0, 255]; 256]);
    assert_eq!(map.chunk(CCoord(1), CCoord(0)), &[[0, 7, 0, 255]; 256]);
}
