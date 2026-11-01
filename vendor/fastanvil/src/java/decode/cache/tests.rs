use super::*;
use mcmap_nbt::{nbt, to_bytes, ByteArray};

fn encoded(name: &str, padding: usize) -> Vec<u8> {
    to_bytes(&nbt!({"block":{
        "Name": name, "unused": ByteArray::new(vec![0; padding])
    }}))
    .unwrap()
}

fn parse(cache: &mut BlockCache, bytes: &[u8]) -> Block {
    let mut cursor = Cursor::root(bytes).unwrap();
    let (tag, _) = cursor.field().unwrap().unwrap();
    let block = cache.parse(&mut cursor, tag).unwrap();
    assert!(cursor.field().unwrap().is_none());
    block
}

#[test]
fn large_keys_bypass_cache_and_total_key_bytes_stay_bounded() {
    let mut cache = BlockCache::default();
    let block = parse(&mut cache, &encoded("test:large", MAX_ENTRY_BYTES));
    assert_eq!(block.name(), "test:large");
    assert_eq!(cache.bytes, 0);
    let mut last_bytes = Vec::new();
    for index in 0..150 {
        let name = format!("test:{index}");
        last_bytes = encoded(&name, 60_000);
        assert_eq!(parse(&mut cache, &last_bytes).name(), name);
        assert!(cache.bytes <= MAX_KEY_BYTES);
        assert!(cache.maps.iter().map(|m| m.len()).sum::<usize>() <= MAX_ENTRIES);
    }
    let bytes_before = cache.bytes;
    assert_eq!(parse(&mut cache, &last_bytes).name(), "test:149");
    assert_eq!(cache.bytes, bytes_before);
}
