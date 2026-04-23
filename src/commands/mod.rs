// Command entry points — one module per subcommand.

pub mod analyze;
pub mod gen_palette;
pub mod render;
pub mod util;

/// Save image as PNG to file or stdout.
///
/// `output_path == "-"` writes a PNG-encoded byte stream to stdout (useful
/// for piping into HTTP handlers etc.); anything else is a filesystem path.
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
