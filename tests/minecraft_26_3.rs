use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use mcmap_nbt::{Value, nbt};
use serde_json::json;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mcmap-26-3-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn region(&self, chunks: &[(usize, i32, Value)]) -> Vec<u8> {
        let mut region = fastanvil::Region::create(Cursor::new(Vec::new())).unwrap();
        for (x, version, palette) in chunks {
            let chunk = nbt!({
                "DataVersion": *version,
                "Status": "minecraft:full",
                "sections": [{
                    "Y": 0_i8,
                    "block_states": {"palette": palette.clone()},
                    "biomes": {"palette": ["minecraft:plains"]}
                }]
            });
            region
                .write_chunk(*x, 0, &mcmap_nbt::to_bytes(&chunk).unwrap())
                .unwrap();
        }
        let bytes = region.into_inner().unwrap().into_inner();
        fs::write(self.0.join("r.0.0.mca"), &bytes).unwrap();
        bytes
    }

    fn palette(&self) {
        fs::write(
            self.0.join("palette.json"),
            json!({
                "minecraft:oak_log|axis=y": [40, 150, 60, 255],
                "minecraft:oak_log|axis=x": [160, 50, 60, 255],
                "minecraft:oak_log|axis=z": [50, 60, 160, 255],
                "minecraft:redstone_lamp|lit=false": [180, 90, 20, 255],
                "minecraft:oak_stairs|facing=east,half=bottom,shape=straight": [60, 190, 140, 255]
            })
            .to_string(),
        )
        .unwrap();
    }

    fn run(&self, command: &str, extra: &[&str]) -> (bool, Vec<serde_json::Value>) {
        let output = Command::new(env!("CARGO_BIN_EXE_mcmap"))
            .current_dir(&self.0)
            .args(["--json", command, "-r", "r.0.0.mca", "-p", "palette.json"])
            .args(extra)
            .output()
            .unwrap();
        if !output.status.success() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
        let events = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        (output.status.success(), events)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn renders_new_and_old_palettes_with_default_and_explicit_properties() {
    let fixture = Fixture::new();
    fixture.palette();
    let before = fixture.region(&[
        (0, 5023, nbt!(["minecraft:oak_log"])),
        (1, 5023, nbt!([{"": "minecraft:redstone_lamp"}])),
        (
            2,
            5023,
            nbt!([{"id": "minecraft:oak_log", "properties": {"axis": "x"}}]),
        ),
        (
            3,
            3955,
            nbt!([{"Name": "minecraft:oak_log", "Properties": {"axis": "z"}}]),
        ),
        (
            4,
            5023,
            nbt!([{"id": "minecraft:oak_stairs", "properties": {"facing": "east"}}]),
        ),
        (5, 5023, nbt!([{"id": "oak_log"}])),
    ]);
    let (success, events) =
        fixture.run("render", &["-o", "tiles", "--split", "--calculate-heights"]);
    assert!(success, "{events:?}");
    assert!(events.iter().any(|e| e["status"] == "rendered"));
    let image = image::open(fixture.0.join("tiles/r.0.0.png"))
        .unwrap()
        .into_rgba8();
    // Flat terrain applies the renderer's 220/255 top shade.
    for (chunk, expected) in [
        [34, 129, 51, 255],
        [155, 77, 17, 255],
        [138, 43, 51, 255],
        [43, 51, 138, 255],
        [51, 163, 120, 255],
        [34, 129, 51, 255],
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            &image.get_pixel(chunk as u32 * 16 + 8, 8).0,
            expected,
            "chunk {chunk}"
        );
    }
    assert_eq!(image.get_pixel(120, 120).0[3], 0);
    assert_eq!(fs::read(fixture.0.join("r.0.0.mca")).unwrap(), before);
}

#[test]
fn mixed_string_wrappers_and_nondefault_states_keep_palette_indices() {
    let fixture = Fixture::new();
    fixture.palette();
    let palette =
        nbt!([{"": "minecraft:oak_log"}, {"id": "minecraft:oak_log", "properties": {"axis": "x"}}]);
    let chunk = nbt!({
        "DataVersion": 5023, "Status": "minecraft:full",
        "sections": [{"Y": 0_i8, "biomes": {"palette": ["minecraft:plains"]}, "block_states": {
            "palette": palette,
            "data": mcmap_nbt::LongArray::new(vec![0x1111111100000000_i64; 256])
        }}]
    });
    let mut region = fastanvil::Region::create(Cursor::new(Vec::new())).unwrap();
    region
        .write_chunk(0, 0, &mcmap_nbt::to_bytes(&chunk).unwrap())
        .unwrap();
    fs::write(
        fixture.0.join("r.0.0.mca"),
        region.into_inner().unwrap().into_inner(),
    )
    .unwrap();
    let (success, events) = fixture.run("render", &["-o", "map.png", "--calculate-heights"]);
    assert!(success, "{events:?}");
    let image = image::open(fixture.0.join("map.png")).unwrap().into_rgba8();
    assert_eq!(image.get_pixel(2, 8).0, [34, 129, 51, 255]);
    assert_eq!(image.get_pixel(12, 8).0, [138, 43, 51, 255]);
}

#[test]
fn analyzes_blocks_saved_as_strings() {
    let fixture = Fixture::new();
    fixture.palette();
    fixture.region(&[(0, 5023, nbt!(["minecraft:diamond_block"]))]);
    let (success, events) = fixture.run("analyze", &["--show-counts"]);
    assert!(success, "{events:?}");
    let result = events.last().unwrap();
    assert_eq!(result["unique_blocks"], 1);
    assert_eq!(
        result["unknown_blocks"],
        json!([{"name": "minecraft:diamond_block", "count": 4096}])
    );
}

#[test]
fn malformed_palettes_report_region_errors_without_publishing_blank_images() {
    for extra in [vec!["-o", "tiles", "--split"], vec!["-o", "map.png"]] {
        let fixture = Fixture::new();
        fixture.palette();
        fixture.region(&[(0, 5023, nbt!([{"properties": {"axis": "y"}}]))]);
        let (success, events) = fixture.run("render", &extra);
        assert!(!success, "{events:?}");
        let region = events
            .iter()
            .find(|event| event["type"] == "region")
            .unwrap();
        assert_eq!(region["status"], "error");
        assert!(region["error"].as_str().unwrap().contains("palette"));
        assert_eq!(events.last().unwrap()["type"], "error");
        assert!(!fixture.0.join("tiles/r.0.0.png").exists());
        assert!(!fixture.0.join("map.png").exists());
    }
}

#[test]
fn empty_regions_are_valid_transparent_maps() {
    let fixture = Fixture::new();
    fixture.palette();
    let mut file = fs::File::create(fixture.0.join("r.0.0.mca")).unwrap();
    file.write_all(&[0; 8192]).unwrap();
    let (success, events) = fixture.run("render", &["-o", "tiles", "--split"]);
    assert!(success, "{events:?}");
    let image = image::open(fixture.0.join("tiles/r.0.0.png"))
        .unwrap()
        .into_rgba8();
    assert!(image.pixels().all(|pixel| pixel.0[3] == 0));
}
