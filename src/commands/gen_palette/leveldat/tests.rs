use super::*;
use flate2::{Compression, write::GzEncoder};
use mcmap_nbt::{Value, nbt, to_bytes};
use std::{
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct File(PathBuf);
impl File {
    fn new(value: Value) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcmap-level-nbt-{}-{}.dat",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&to_bytes(&value).unwrap()).unwrap();
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        Self(path)
    }
}
impl Drop for File {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn nbt_registry_detection_and_legacy_ids_are_preserved() {
    let legacy = File::new(nbt!({"FML":{"ItemData":[
        {"K":"\u{1}minecraft:stone","V":1},
        {"K":"\u{1}mod:扩展","V":4097},
        {"K":"\u{2}mod:item","V":32000},
        {"K":"\u{1}mod:invalid","V":-1}
    ]},"Data":{"ignored":[L;1,2,3]}}));
    assert_eq!(
        detect_variant(Some(&legacy.0)).unwrap(),
        PaletteVariant::Legacy
    );
    let registry = load_fml_registry_v17(&legacy.0).unwrap();
    assert_eq!(registry.blocks.len(), 2);
    assert_eq!(registry.blocks[&4097], "mod:扩展");
    assert_eq!(registry.items[&32000], "mod:item");
    let forge = File::new(
        nbt!({"FML":{"ItemData":[],"Registries":{"minecraft:blocks":{"ids":[
            {"K":"minecraft:stone","V":1},{"K":"mod:large","V":70000}
        ]}}}}),
    );
    assert_eq!(
        detect_variant(Some(&forge.0)).unwrap(),
        PaletteVariant::Forge112
    );
    assert_eq!(
        load_fml_registry_v12(&forge.0).unwrap().blocks[&70000],
        "mod:large"
    );
    let modern = File::new(nbt!({"Data":{"DataVersion":5023}}));
    assert_eq!(
        detect_variant(Some(&modern.0)).unwrap(),
        PaletteVariant::Modern
    );
}
