use super::a_over_b_colour;

#[test]
fn current_pixel_failure_does_not_evaluate_northern_heights() {
    struct FailingPalette;
    impl crate::Palette for FailingPalette {
        fn pick(&self, _: &crate::Block, _: Option<crate::biome::Biome>) -> [u8; 4] {
            panic!("current pixel cannot be rendered")
        }
    }
    let bytes = mcmap_nbt::to_bytes(&mcmap_nbt::nbt!({
        "DataVersion": 5023,
        "Status": "minecraft:full",
        "sections": [{"Y": 0_i8, "block_states": {"palette": ["minecraft:stone"]}}],
    }))
    .unwrap();
    let chunk = crate::JavaChunk::from_bytes(&bytes).unwrap();
    let north_queried = std::cell::Cell::new(false);
    let renderer = crate::TopShadeRenderer::new(&FailingPalette, crate::HeightMode::Trust);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.render_with_north_heights(&chunk, || {
            north_queried.set(true);
            Some([16; 16])
        })
    }));
    assert!(result.is_err());
    assert!(!north_queried.get());
}

fn a_over_b_colour_general(colour: [u8; 4], below_colour: [u8; 4]) -> [u8; 4] {
    let linear = |c: u8| (((c as usize).pow(2)) as f32) / ((255 * 255) as f32);
    let colour = colour.map(linear);
    let below_colour = below_colour.map(linear);
    let over_component = |ca: f32, aa: f32, cb: f32, ab: f32| {
        let a_out = aa + ab * (1. - aa);
        let linear_out = (ca * aa + cb * ab * (1. - aa)) / a_out;
        (linear_out * 255. * 255.).sqrt() as u8
    };
    let over_alpha = |aa: f32, ab: f32| {
        let a_out = aa + ab * (1. - aa);
        (a_out * 255. * 255.).sqrt() as u8
    };
    [
        over_component(colour[0], colour[3], below_colour[0], below_colour[3]),
        over_component(colour[1], colour[3], below_colour[1], below_colour[3]),
        over_component(colour[2], colour[3], below_colour[2], below_colour[3]),
        over_alpha(colour[3], below_colour[3]),
    ]
}

#[test]
fn transparent_first_layer_preserves_every_channel_and_alpha() {
    for alpha in 0..=255_u8 {
        for channel in 0..=255_u8 {
            let below = [channel, channel.wrapping_mul(83), 255 - channel, alpha];
            assert_eq!(
                a_over_b_colour([0; 4], below),
                a_over_b_colour_general([0; 4], below)
            );
        }
    }
}

#[test]
fn opaque_background_preserves_channel_pair_rounding() {
    for alpha in 0..=255_u8 {
        let linear_alpha = (alpha as usize).pow(2) as f32 / 65025.0;
        assert_eq!(linear_alpha + 1.0 * (1.0 - linear_alpha), 1.0);
    }
    for alpha in [1, 2, 64, 128, 180, 250, 254, 255] {
        for top in 0..=255_u8 {
            for below in 0..=255_u8 {
                let a = [top, 255 - top, top.wrapping_mul(83), alpha];
                let b = [below, 255 - below, below.wrapping_mul(83), 255];
                assert_eq!(a_over_b_colour(a, b), a_over_b_colour_general(a, b));
            }
        }
    }
}

#[test]
fn first_opaque_colour_preserves_every_channel_rounding() {
    for channel in 0..=255_u8 {
        let below = [channel, channel.wrapping_mul(83), 255 - channel, 255];
        for top in [[0; 4], [255, 127, 1, 0], [channel, 255 - channel, 47, 0]] {
            assert_eq!(
                a_over_b_colour(top, below),
                a_over_b_colour_general(top, below),
                "top={top:?}, below={below:?}"
            );
        }
    }
    assert_eq!(
        a_over_b_colour([0; 4], [47, 128, 228, 255]),
        [46, 127, 227, 255]
    );
}

#[test]
fn partial_transparency_keeps_original_composition() {
    for alpha in 0..=255_u8 {
        for below_alpha in 0..=255_u8 {
            for channel in [0, 1, 47, 128, 255] {
                let top = [channel, 255 - channel, 127, alpha];
                let below = [255 - channel, channel, 228, below_alpha];
                assert_eq!(
                    a_over_b_colour(top, below),
                    a_over_b_colour_general(top, below),
                    "top={top:?}, below={below:?}"
                );
            }
        }
    }
}
