use fastnbt::{nbt, Value};
use serde::Deserialize;

use super::{Block, BlockArchetype};

fn palette(value: Value) -> fastnbt::error::Result<Vec<Block>> {
    #[derive(Deserialize)]
    struct Section {
        palette: Vec<Block>,
    }
    let bytes = fastnbt::to_bytes(&nbt!({ "palette": value })).unwrap();
    fastnbt::from_bytes::<Section>(&bytes).map(|section| section.palette)
}

#[test]
fn legacy_states_preserve_properties_and_rendering_archetypes() {
    let blocks = palette(nbt!([
        {"Name": "minecraft:oak_log"},
        {"Name": "minecraft:oak_log", "Properties": {"axis": "x", "waterlogged": "true", "powered": "true"}},
        {"Name": "minecraft:grass_block", "Properties": {"snowy": "true"}},
        {"Name": "minecraft:water"},
        {"Name": "minecraft:air"}
    ])).unwrap();
    assert_eq!(blocks[0].encoded_description(), "minecraft:oak_log|");
    assert_eq!(blocks[1].encoded_description(), "minecraft:oak_log|axis=x");
    assert!(blocks[2].snowy());
    assert_eq!(blocks[3].archetype, BlockArchetype::Watery);
    assert_eq!(blocks[4].archetype, BlockArchetype::Airy);
}

#[test]
fn modern_defaults_wrappers_and_overrides_match_explicit_states() {
    let strings = palette(nbt!([
        "oak_log",
        "minecraft:redstone_torch",
        "minecraft:oak_stairs"
    ]))
    .unwrap();
    assert_eq!(strings[0].name(), "minecraft:oak_log");
    assert_eq!(strings[0].encoded_description(), "minecraft:oak_log|axis=y");
    assert_eq!(
        strings[1].encoded_description(),
        "minecraft:redstone_torch|lit=true"
    );
    assert_eq!(
        strings[2].encoded_description(),
        "minecraft:oak_stairs|facing=north,half=bottom,shape=straight"
    );
    let blocks = palette(nbt!([
        {"": "minecraft:oak_log"},
        {"properties": {"axis": "x"}, "id": "minecraft:oak_log"},
        {"id": "minecraft:oak_stairs", "properties": {"facing": "east", "waterlogged": "true"}},
        {"id": "minecraft:grass_block", "properties": {"snowy": "true"}}
    ]))
    .unwrap();
    assert_eq!(
        blocks[0].encoded_description(),
        strings[0].encoded_description()
    );
    assert_eq!(blocks[1].encoded_description(), "minecraft:oak_log|axis=x");
    assert_eq!(
        blocks[2].encoded_description(),
        "minecraft:oak_stairs|facing=east,half=bottom,shape=straight"
    );
    assert!(blocks[3].snowy());
}

#[test]
fn modded_and_unicode_states_preserve_explicit_properties() {
    let blocks = palette(nbt!([
        {"id": "example:café🔧", "properties": {"z": "非默认", "a": "é"}, "metadata": {"unused": [L; 1, 2, 3]}},
        {"Name": "minecraft:oak_log", "id": "minecraft:stone", "Properties": {"axis": "z"}}
    ])).unwrap();
    assert_eq!(blocks[0].name(), "example:café🔧");
    assert_eq!(
        blocks[0].encoded_description(),
        "example:café🔧|a=é,z=非默认"
    );
    assert_eq!(blocks[1].encoded_description(), "minecraft:oak_log|axis=z");
}

#[test]
fn malformed_states_fail_instead_of_becoming_air() {
    for value in [
        nbt!([{"properties": {"axis": "x"}}]),
        nbt!([{"id": 1}]),
        nbt!([{"": 1}]),
        nbt!([{"id": "minecraft:oak_log", "properties": "axis=x"}]),
        nbt!([{"id": "minecraft:oak_log", "properties": {"axis": 1}}]),
        nbt!([1]),
    ] {
        assert!(palette(value).is_err());
    }
}
