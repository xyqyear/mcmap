use super::ChunkDecoder;
use crate::{Chunk, JavaChunk};
use mcmap_nbt::{nbt, to_bytes};

fn chunk(block: mcmap_nbt::Value) -> Vec<u8> {
    to_bytes(&nbt!({
        "DataVersion": 5023, "Status": "full",
        "sections": [{"Y": 0i8, "block_states": {"palette": [block]}}],
    }))
    .unwrap()
}

fn block_name(chunk: &JavaChunk) -> &str {
    chunk.block(0, 0, 0).unwrap().name()
}

#[test]
fn reused_decoder_recovers_after_errors_and_keeps_owned_chunks() {
    let mut decoder = ChunkDecoder::default();
    let mut bytes = chunk(nbt!("minecraft:stone"));
    let retained = decoder.decode(&bytes).unwrap();
    for end in 0..bytes.len() {
        assert!(decoder.decode(&bytes[..end]).is_err());
    }
    bytes.fill(0);
    let next = decoder
        .decode(&chunk(
            nbt!({"id":"minecraft:oak_log","properties":{"axis":"x"}}),
        ))
        .unwrap();
    drop(decoder);
    assert_eq!(block_name(&retained), "minecraft:stone");
    assert_eq!(block_name(&next), "minecraft:oak_log");
    assert!(next
        .block(0, 0, 0)
        .unwrap()
        .encoded_description()
        .contains("axis=x"));
}

#[test]
fn cache_eviction_and_separate_regions_preserve_block_properties() {
    let mut decoder = ChunkDecoder::default();
    let first = decoder
        .decode(&chunk(
            nbt!({"Name":"test:first","Properties":{"key":"old"}}),
        ))
        .unwrap();
    for index in 0..4200 {
        let name = format!("test:block_{index}");
        let decoded = decoder.decode(&chunk(nbt!({"Name":name.clone()}))).unwrap();
        assert_eq!(block_name(&decoded), name);
    }
    let bytes = chunk(nbt!({"Name":"test:first","Properties":{"key":"new"}}));
    let last = decoder.decode(&bytes).unwrap();
    let independent = ChunkDecoder::default().decode(&bytes).unwrap();
    assert_eq!(format!("{last:?}"), format!("{independent:?}"));
    assert_eq!(
        first.block(0, 0, 0).unwrap().encoded_description(),
        "test:first|key=old"
    );
    assert_eq!(
        last.block(0, 0, 0).unwrap().encoded_description(),
        "test:first|key=new"
    );
}
