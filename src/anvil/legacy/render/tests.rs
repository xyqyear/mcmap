use super::{LegacyTopShadeRenderer, PaletteFormat, RenderEngine};
use crate::anvil::legacy::{
    LegacyPalette,
    chunk::{LegacyChunkData, LegacySection},
};

fn empty_chunk(heightmap: Option<[i32; 256]>) -> LegacyChunkData {
    LegacyChunkData {
        x_pos: 0,
        z_pos: 0,
        sections: Default::default(),
        biomes: None,
        heightmap,
    }
}

fn set(chunk: &mut LegacyChunkData, x: usize, y: usize, z: usize, id: u16, meta: u16) {
    let section = chunk.sections[y / 16].get_or_insert_with(|| LegacySection {
        y: (y / 16) as i8,
        ids: Box::new([0; 4096]),
        metas: Box::new([0; 4096]),
    });
    let index = ((y % 16) * 256) + z * 16 + x;
    section.ids[index] = id;
    section.metas[index] = meta;
}

#[test]
fn transparent_and_unknown_ids_still_cast_northern_shading() {
    let palette = LegacyPalette::from_slice(
        br#"{"format":"1.7.10","blocks":{
        "1":[200,100,60,255],"3":[30,40,50,0]
    }}"#,
    )
    .unwrap();
    let renderer = LegacyTopShadeRenderer::new(&palette, PaletteFormat::Legacy17);
    for heightmap in [None, Some([32; 256])] {
        let mut chunk = empty_chunk(heightmap);
        set(&mut chunk, 0, 20, 0, 2, 0);
        set(&mut chunk, 0, 10, 0, 1, 0);
        set(&mut chunk, 0, 10, 1, 1, 0);
        set(&mut chunk, 0, 10, 3, 1, 0);
        set(&mut chunk, 1, 20, 0, 3, 0);
        set(&mut chunk, 1, 9, 0, 1, 0);
        set(&mut chunk, 1, 10, 1, 1, 0);
        let pixels = renderer.render_chunk(chunk, None).0;
        assert_eq!(pixels[0], [172, 86, 51, 255]);
        assert_eq!(pixels[16], [141, 70, 42, 255]);
        assert_eq!(pixels[32], [0, 0, 0, 0]);
        assert_eq!(pixels[48], [172, 86, 51, 255]);
        assert_eq!(pixels[17], [141, 70, 42, 255]);
    }
}

#[test]
fn northern_chunk_and_translucent_composition_remain_unchanged() {
    let palette = LegacyPalette::from_slice(
        br#"{"format":"1.7.10","blocks":{
        "1":[200,100,60,255],"4":[200,0,0,128],"5":[0,0,200,255]
    }}"#,
    )
    .unwrap();
    let renderer = LegacyTopShadeRenderer::new(&palette, PaletteFormat::Legacy17);
    let mut chunk = empty_chunk(Some([32; 256]));
    let mut north = empty_chunk(Some([32; 256]));
    set(&mut north, 0, 22, 15, 2, 0);
    set(&mut chunk, 0, 12, 0, 1, 0);
    set(&mut chunk, 1, 12, 0, 4, 0);
    set(&mut chunk, 1, 10, 0, 5, 0);
    let pixels = renderer
        .render_chunk(chunk, Some(&renderer.boundary(north)))
        .0;
    assert_eq!(pixels[0], [141, 70, 42, 255]);
    assert_eq!(pixels[1], [86, 0, 86, 255]);
    assert_eq!(pixels[2], [0, 0, 0, 0]);
}

#[test]
fn cached_boundary_keeps_raw_heights_and_empty_columns() {
    let palette =
        LegacyPalette::from_slice(br#"{"format":"1.7.10","blocks":{"1":[200,100,60,255]}}"#)
            .unwrap();
    let renderer = LegacyTopShadeRenderer::new(&palette, PaletteFormat::Legacy17);
    let make_north = || {
        let mut north = empty_chunk(Some([32; 256]));
        set(&mut north, 0, 22, 15, 2, 0);
        set(&mut north, 0, 10, 15, 1, 0);
        north
    };
    let make_south = || {
        let mut chunk = empty_chunk(Some([32; 256]));
        set(&mut chunk, 0, 12, 0, 1, 0);
        set(&mut chunk, 1, 12, 0, 1, 0);
        chunk
    };
    let seed = renderer.boundary(make_north());
    let (_, cached) = renderer.render_chunk(make_north(), None);
    let super::NorthBoundary::Heights(heights) = &cached else {
        panic!("full chunk retained")
    };
    assert_eq!(heights[0], Some(22));
    assert_eq!(heights[1], None);
    let expected = renderer.render_chunk(make_south(), Some(&seed)).0;
    assert_eq!(expected[0], [141, 70, 42, 255]);
    assert_eq!(expected[1], [172, 86, 51, 255]);
    assert_eq!(
        renderer.render_chunk(make_south(), Some(&cached)).0,
        expected
    );
}
