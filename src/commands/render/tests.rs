use super::save_image;
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

struct Output(PathBuf);

impl Output {
    fn new(extension: &str) -> Self {
        Self(std::env::temp_dir().join(format!("mcmap-image-{}.{}", std::process::id(), extension)))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn png_preserves_every_alpha_and_channel_value() {
    let pixels = RgbaImage::from_fn(256, 256, |x, y| {
        Rgba([x as u8, (255 - x) as u8, (x ^ y) as u8, y as u8])
    });
    let output = Output::new("PNG");
    save_image(&pixels, &output.0).unwrap();
    assert_eq!(image::open(&output.0).unwrap().to_rgba8(), pixels);
}

#[test]
fn output_extension_still_selects_other_encoders() {
    let pixels = RgbaImage::from_fn(16, 16, |x, y| {
        Rgba([x as u8 * 16, y as u8 * 16, 42, (x ^ y) as u8 * 16])
    });
    let output = Output::new("tga");
    save_image(&pixels, &output.0).unwrap();
    assert_eq!(image::open(&output.0).unwrap().to_rgba8(), pixels);
    let unsupported = Output::new("unknown");
    let original_error = pixels.save(&unsupported.0).unwrap_err().to_string();
    assert_eq!(
        save_image(&pixels, &unsupported.0).unwrap_err().to_string(),
        original_error
    );
}
