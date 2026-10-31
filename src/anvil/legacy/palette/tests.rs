use super::{LegacyPalette, LegacyPaletteFile};

#[test]
fn exact_metadata_preserves_transparent_overrides_and_large_values() {
    let palette = LegacyPalette::from_slice(
        br#"{
        "format":"1.7.10",
        "blocks":{
            "0":[10,20,30,255],
            "1":[20,40,60,255],
            "1|0":[1,2,3,0],
            "1|15":[15,30,45,255],
            "1|16":[16,32,48,255],
            "1|65535":[255,127,63,0],
            "65535|15":[80,90,100,255],
            "65535|65535":[150,160,170,255]
        }
    }"#,
    )
    .unwrap();
    assert_eq!(palette.len(), 8);
    for meta in [0, 15, 16, 65535] {
        assert_eq!(palette.lookup(0, meta), [10, 20, 30, 255]);
        assert_eq!(palette.lookup(50000, meta), [0, 0, 0, 0]);
    }
    assert_eq!(palette.lookup(1, 0), [1, 2, 3, 0]);
    assert_eq!(palette.lookup(1, 15), [15, 30, 45, 255]);
    assert_eq!(palette.lookup(1, 16), [16, 32, 48, 255]);
    assert_eq!(palette.lookup(1, 65535), [255, 127, 63, 0]);
    assert_eq!(palette.lookup(1, 17), [20, 40, 60, 255]);
    assert_eq!(palette.lookup(65535, 0), [0, 0, 0, 0]);
    assert_eq!(palette.lookup(65535, 15), [80, 90, 100, 255]);
    assert_eq!(palette.lookup(65535, 16), [0, 0, 0, 0]);
    assert_eq!(palette.lookup(65535, 65535), [150, 160, 170, 255]);
}

#[test]
fn all_u16_ids_can_have_fallback_colours() {
    let blocks = (0..=u16::MAX)
        .map(|id| (id.to_string(), [(id >> 8) as u8, id as u8, 13, 255]))
        .collect();
    let palette = LegacyPalette::from_file(LegacyPaletteFile {
        format: "1.12.2".into(),
        blocks,
    });
    assert_eq!(palette.len(), 65536);
    for id in 0..=u16::MAX {
        let expected = [(id >> 8) as u8, id as u8, 13, 255];
        for meta in [0, 15, 16, 65535] {
            assert_eq!(palette.lookup(id, meta), expected);
        }
    }
}

#[test]
fn malformed_keys_are_ignored_and_numeric_aliases_count_once() {
    let palette = LegacyPalette::from_slice(
        br#"{
        "format":"1.12.2",
        "blocks":{
            "1":[20,40,60,255], "01":[20,40,60,255],
            "1|0":[1,2,3,0], "01|00":[1,2,3,0],
            "":[255,0,0,255], "-1":[255,0,0,255], "65536":[255,0,0,255],
            "1|65536":[255,0,0,255], "1|x":[255,0,0,255], "1|2|3":[255,0,0,255]
        }
    }"#,
    )
    .unwrap();
    assert_eq!(palette.len(), 2);
    assert_eq!(palette.lookup(1, 0), [1, 2, 3, 0]);
    assert_eq!(palette.lookup(1, 12), [20, 40, 60, 255]);
    assert_eq!(palette.lookup(65535, 65535), [0, 0, 0, 0]);
}

#[test]
fn empty_palettes_and_invalid_formats_keep_their_behavior() {
    for format in ["1.7.10", "1.12.2"] {
        let bytes = format!(r#"{{"format":"{format}","blocks":{{}}}}"#);
        let palette = LegacyPalette::from_slice(bytes.as_bytes()).unwrap();
        assert_eq!(palette.len(), 0);
        assert_eq!(palette.lookup(0, 0), [0, 0, 0, 0]);
        assert_eq!(palette.lookup(65535, 65535), [0, 0, 0, 0]);
    }
    let error = LegacyPalette::from_slice(br#"{"format":"1.13","blocks":{}}"#).unwrap_err();
    assert!(error.to_string().contains("Expected legacy palette"));
    assert!(LegacyPalette::from_slice(b"{").is_err());
}
