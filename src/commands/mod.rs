// Command modules

pub mod analyze;
pub mod gen_palette;
pub mod heightmap;
pub mod render;

/// Save image as PNG to file or stdout
///
/// # Arguments
/// * `img` - Image buffer
/// * `output_path` - Output file path, or "-" for stdout
pub fn save_png(
    img: image::RgbaImage,
    output_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if output_path == "-" {
        use image::ImageEncoder;
        use std::io::Write;
        let mut buffer = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
        encoder.write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ColorType::Rgba8,
        )?;
        std::io::stdout().write_all(&buffer)?;
    } else {
        img.save(output_path)?;
    }

    Ok(())
}
