use serde::Deserialize;

use super::{AnyPalette, LegacyPalette, PaletteFormat, RenderedPalette, Result, from_slice};

fn reference(bytes: &[u8]) -> Result<AnyPalette> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        format: Option<String>,
    }
    let probe: Probe = serde_json::from_slice(bytes)?;
    let format = match probe.format.as_deref() {
        Some("1.7.10") => PaletteFormat::Legacy17,
        Some("1.12.2") => PaletteFormat::Forge112,
        _ => PaletteFormat::Modern,
    };
    match format {
        PaletteFormat::Modern => Ok(AnyPalette::Modern(RenderedPalette::new(
            serde_json::from_slice(bytes)?,
        ))),
        _ => Ok(AnyPalette::Legacy(
            LegacyPalette::from_slice(bytes)?,
            format,
        )),
    }
}

#[derive(Debug, PartialEq)]
enum Snapshot {
    Modern(Vec<(String, [u8; 4])>),
    Legacy(PaletteFormat, usize, Vec<[u8; 4]>),
}

fn snapshot(palette: Result<AnyPalette>) -> std::result::Result<Snapshot, String> {
    match palette.map_err(|error| error.to_string())? {
        AnyPalette::Modern(palette) => {
            let mut colors: Vec<_> = palette.blockstates.into_iter().collect();
            colors.sort_unstable();
            Ok(Snapshot::Modern(colors))
        }
        AnyPalette::Legacy(palette, format) => {
            let mut colors = Vec::new();
            for id in [0, 1, 2, 9, 65535] {
                for meta in [0, 1, 15, 16, 65535] {
                    colors.push(palette.lookup(id, meta));
                }
            }
            Ok(Snapshot::Legacy(format, palette.len(), colors))
        }
    }
}

fn assert_matches_reference(bytes: &[u8]) {
    assert_eq!(
        snapshot(from_slice(bytes)),
        snapshot(reference(bytes)),
        "{}",
        String::from_utf8_lossy(bytes)
    );
}

#[test]
fn palette_dispatch_preserves_maps_wrappers_and_ignored_fields() {
    for input in [
        r#"{}"#,
        r#"{"minecraft:stone":[1,2,3,4],"example:é💧":[255,0,128,64]}"#,
        r#"{"minecraft:stone":[1,2,3,4],"minecraft:stone":[5,6,7,8]}"#,
        r#"{"format":"1.7.10","blocks":{"1":[1,2,3,4],"1|15":[5,6,7,8],"65535|65535":[9,10,11,12]}}"#,
        r#"{"blocks":{"2":[100,110,120,255]},"format":"1.12.2"}"#,
        r#"{"ignored":[true,null,{"nested":[[1],"text"]}],"format":"1.7.10","blocks":{}}"#,
        r#"{"minecraft:stone":[1,2,3,4],"format":"1.12.2","blocks":{},"other":false}"#,
    ] {
        assert_matches_reference(input.as_bytes());
    }
}

#[test]
fn palette_dispatch_preserves_format_and_json_error_text() {
    for input in [
        "",
        "null",
        "true",
        "42",
        "[]",
        "[null]",
        "[null,{}]",
        r#"["1.7.10"]"#,
        r#"["1.7.10",{}]"#,
        r#"{"format":[1,2,3,4]}"#,
        r#"{"\u0066ormat":[1,2,3,4]}"#,
        r#"{"format":null,"minecraft:stone":[1,2,3,4]}"#,
        r#"{"format":"unknown"}"#,
        r#"{"format":{},"minecraft:stone":[1,2,3,4]}"#,
        r#"{"format":"1.7.10","format":"1.7.10","blocks":{}}"#,
        r#"{"format":null,"format":null}"#,
        r#"{"format":"1.12.2","blocks":null}"#,
        r#"{"format":"1.7.10"}"#,
        r#"{"format":"1.12.2","blocks":{"1":[0,1,2,256]}}"#,
        r#"{"minecraft:stone":[1,2,3]}"#,
        r#"{"minecraft:stone":[1,2,3,4,5]}"#,
        r#"{"minecraft:stone":[1.0,2,3,4]}"#,
        r#"{"minecraft:stone":[-1,2,3,4]}"#,
        r#"{"minecraft:stone":[1,2,3,256]}"#,
        r#"{"minecraft:stone":null}"#,
        r#"{"minecraft:stone":[1,2,3,4],}"#,
        r#"{"minecraft:stone":[1,2,3,4]} trailing"#,
        "{\n\"minecraft:stone\":[1,2,3,4],\n\"other\": {",
    ] {
        assert_matches_reference(input.as_bytes());
    }
    assert_matches_reference(b"\xef\xbb\xbf{}");
    assert_matches_reference(b"{\"\xff\":[0,0,0,0]}");
}

#[test]
fn duplicate_format_values_keep_probe_validation_order() {
    let values = [
        "null",
        "false",
        "1",
        "\"1.7.10\"",
        "\"1.12.2\"",
        "\"other\"",
        "[1,2,3,4]",
        "{}",
    ];
    for first in values {
        for second in values {
            let input = format!(r#"{{"format":{first},"format":{second},"blocks":{{}}}}"#);
            assert_matches_reference(input.as_bytes());
        }
    }
}
