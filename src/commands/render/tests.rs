use super::save_image;
use image::{Rgba, RgbaImage};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Output(PathBuf);

impl Output {
    fn new(extension: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "mcmap-image-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        )))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::remove_dir(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn assert_rgba_png(output: &Output, pixels: &RgbaImage) {
    let bytes = std::fs::read(&output.0).unwrap();
    let (info, reader) = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    assert_eq!((info.width, info.height), pixels.dimensions());
    assert_eq!(info.color_type, png::ColorType::RGBA);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert!(!reader.info().interlaced);
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().into_rgba8(),
        *pixels
    );
}

#[test]
fn png_preserves_every_alpha_and_channel_value() {
    let pixels = RgbaImage::from_fn(256, 256, |x, y| {
        Rgba([x as u8, (255 - x) as u8, (x ^ y) as u8, y as u8])
    });
    let output = Output::new("PNG");
    save_image(&pixels, &output.0).unwrap();
    assert_rgba_png(&output, &pixels);
}

#[test]
fn small_and_unaligned_png_dimensions_keep_every_pixel() {
    for (width, height) in [(1, 1), (1, 7), (2, 3), (3, 2), (7, 11), (13, 5)] {
        let pixels = RgbaImage::from_fn(width, height, |x, y| {
            Rgba([
                (x * 31) as u8,
                (y * 47) as u8,
                (x ^ y) as u8,
                (x + y * 3) as u8,
            ])
        });
        let output = Output::new("png");
        save_image(&pixels, &output.0).unwrap();
        assert_rgba_png(&output, &pixels);
    }
}

#[test]
fn images_larger_than_sixteen_mib_keep_the_original_encoder() {
    let pixels = RgbaImage::from_pixel(2048, 2049, Rgba([23, 57, 89, 123]));
    let original = Output::new("png");
    let current = Output::new("png");
    pixels.save(&original.0).unwrap();
    save_image(&pixels, &current.0).unwrap();
    assert_eq!(
        std::fs::read(&original.0).unwrap(),
        std::fs::read(&current.0).unwrap()
    );
}

#[test]
fn zero_dimensions_and_extra_raw_data_keep_original_results() {
    for pixels in [
        RgbaImage::new(0, 0),
        RgbaImage::new(0, 7),
        RgbaImage::new(7, 0),
        RgbaImage::from_raw(1, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap(),
    ] {
        let original = Output::new("png");
        let current = Output::new("png");
        let expected = pixels.save(&original.0).map_err(|error| error.to_string());
        let actual = save_image(&pixels, &current.0).map_err(|error| error.to_string());
        assert_eq!(actual, expected);
        assert_eq!(
            std::fs::read(&original.0).unwrap(),
            std::fs::read(&current.0).unwrap()
        );
    }
}

#[test]
fn png_file_creation_failures_are_reported() {
    let pixels = RgbaImage::from_pixel(3, 5, Rgba([1, 2, 3, 4]));
    let missing_parent = Output::new("missing");
    let path = missing_parent.0.join("image.png");
    let expected = pixels.save(&path).unwrap_err().to_string();
    assert_eq!(
        save_image(&pixels, &path).unwrap_err().to_string(),
        expected
    );
    assert!(!path.exists());

    let directory = Output::new("png");
    std::fs::create_dir(&directory.0).unwrap();
    let expected = pixels.save(&directory.0).unwrap_err().to_string();
    assert_eq!(
        save_image(&pixels, &directory.0).unwrap_err().to_string(),
        expected
    );
}

#[test]
fn output_extension_still_selects_other_encoders() {
    let pixels = RgbaImage::from_fn(16, 16, |x, y| {
        Rgba([x as u8 * 16, y as u8 * 16, 42, (x ^ y) as u8 * 16])
    });
    let output = Output::new("tga");
    save_image(&pixels, &output.0).unwrap();
    assert_eq!(image::open(&output.0).unwrap().into_rgba8(), pixels);
    let unsupported = Output::new("unknown");
    let original_error = pixels.save(&unsupported.0).unwrap_err().to_string();
    assert_eq!(
        save_image(&pixels, &unsupported.0).unwrap_err().to_string(),
        original_error
    );
}
