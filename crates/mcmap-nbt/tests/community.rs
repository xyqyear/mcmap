mod support;
use mcmap_nbt::{Value, from_bytes, to_bytes};
use sha1::{Digest, Sha1};
use std::path::Path;

#[test]
fn pinned_community_files_match_independent_reference_and_round_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha1::digest(&bytes)),
            case["input_sha1"].as_str().unwrap(),
            "{name}"
        );
        let value: Value = from_bytes(&bytes).unwrap();
        let canonical = support::canonical(&value);
        assert_eq!(
            format!("{:x}", Sha1::digest(&canonical)),
            case["canonical_sha1"].as_str().unwrap(),
            "{name}"
        );
        let round_trip: Value = from_bytes(&to_bytes(&value).unwrap()).unwrap();
        assert_eq!(support::canonical(&round_trip), canonical, "{name}");
    }
}
