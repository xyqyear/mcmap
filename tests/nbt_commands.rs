use flate2::{Compression, write::GzEncoder};
use mcmap_nbt::{Value, nbt, to_bytes};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

struct World(PathBuf);
impl World {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "mcmap-nbt-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, value: Value, gzip: bool) -> Vec<u8> {
        let mut bytes = to_bytes(&value).unwrap();
        if gzip {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&bytes).unwrap();
            bytes = encoder.finish().unwrap();
        }
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &bytes).unwrap();
        bytes
    }
    fn run(&self, command: &str) -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_mcmap"))
            .args(["--json", command, "--world"])
            .arg(&self.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|event| event["type"] == "result")
            .unwrap()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn per_team_claims_keep_members_dimensions_and_force_load_flags() {
    let world = World::new();
    let path = "serverutilities/teams/claimedchunks/alice.dat";
    let before = world.write(
        path,
        nbt!({"ClaimedChunks":{"-1":[{"x":-3,"z":8,"loaded":1i8}]}}),
        true,
    );
    world.write(
        "serverutilities/teams/alice.dat",
        nbt!({"Type":"player","Owner":"Alice","Title":"中文🦀","Players":{"Bob":"member"}}),
        false,
    );
    world.write(
        "serverutilities/players/alice.dat",
        nbt!({"Name":"Alice","UUID":"01234567-89ab-cdef-0123-456789abcdef"}),
        true,
    );
    let output = world.run("extract-ftb-claims");
    assert_eq!(output["detected_format"], "per_team_nbt");
    assert_eq!(output["claims"], 1);
    let team = &output["data"]["teams"][0];
    assert_eq!(team["name"], "中文🦀");
    assert_eq!(team["claims"][0]["dim"], "-1");
    assert_eq!(team["claims"][0]["cx"], -3);
    assert_eq!(team["claims"][0]["cz"], 8);
    assert_eq!(team["claims"][0]["force_loaded"], true);
    assert_eq!(
        team["owner"]["uuid"],
        "01234567-89ab-cdef-0123-456789abcdef"
    );
    assert_eq!(team["members"].as_array().unwrap().len(), 2);
    assert_eq!(fs::read(world.0.join(path)).unwrap(), before);
}

#[test]
fn universe_claims_keep_int_array_entries_and_skip_invalid_entries() {
    let world = World::new();
    let owner = "01234567-89ab-cdef-0123-456789abcdef";
    let path = "data/ftb_lib/universe.dat";
    let before = world.write(
        path,
        nbt!({"Data":{"ftbu:data":{"Chunks":{
            owner: [[I;0,-9,12,1],[I;-1,4,-6,0],[I;1,2]]
        }}}}),
        true,
    );
    world.write(
        &format!("data/ftb_lib/players/{owner}.dat"),
        nbt!({"Name":"Alice","TeamID":""}),
        false,
    );
    let output = world.run("extract-ftb-claims");
    assert_eq!(output["detected_format"], "universe_dat");
    assert_eq!(output["claims"], 2);
    let claims = output["data"]["teams"][0]["claims"].as_array().unwrap();
    assert!(
        claims.iter().any(|c| c["dim"] == "0"
            && c["cx"] == -9
            && c["cz"] == 12
            && c["force_loaded"] == true)
    );
    assert!(
        claims.iter().any(|c| c["dim"] == "-1"
            && c["cx"] == 4
            && c["cz"] == -6
            && c["force_loaded"] == false)
    );
    assert_eq!(fs::read(world.0.join(path)).unwrap(), before);
}

#[test]
fn player_extraction_keeps_coordinates_and_reports_corrupt_files() {
    let world = World::new();
    let path = "playerdata/01234567-89ab-cdef-0123-456789abcdef.dat";
    let before=world.write(path,nbt!({"DataVersion":5023,"Dimension":"minecraft:overworld","Pos":[1.25f64,64.0f64,-2.5f64],"Inventory":[{"ignored":"🦀"}]}),true);
    fs::write(
        world
            .0
            .join("playerdata/11234567-89ab-cdef-0123-456789abcdef.dat"),
        [10, 0],
    )
    .unwrap();
    let output = world.run("extract-players");
    assert_eq!(output["players"], 1);
    assert_eq!(output["skipped"], 1);
    let player = &output["data"]["players"][0];
    assert_eq!(player["data_version"], 5023);
    assert_eq!(player["dim"], "minecraft:overworld");
    assert_eq!(player["pos"]["x"], 1.25);
    assert_eq!(player["pos"]["y"], 64.0);
    assert_eq!(player["pos"]["z"], -2.5);
    assert_eq!(fs::read(world.0.join(path)).unwrap(), before);
}
