use std::{path::Path, ptr::NonNull};

use image::{ImageFormat, ImageResult, RgbaImage};
use libdeflate_sys::{
    libdeflate_alloc_compressor, libdeflate_compressor, libdeflate_free_compressor,
    libdeflate_zlib_compress, libdeflate_zlib_compress_bound,
};

const MAX_NATIVE_RGBA_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn save_image(image: &RgbaImage, path: &Path) -> ImageResult<()> {
    if matches!(ImageFormat::from_path(path), Ok(ImageFormat::Png))
        && let Some(encoded) = encode_native(image)
    {
        std::fs::write(path, encoded)?;
        return Ok(());
    }
    image.save(path)
}

fn encode_native(image: &RgbaImage) -> Option<Vec<u8>> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let row = usize::try_from(width).ok()?.checked_mul(4)?;
    let height_bytes = usize::try_from(height).ok()?;
    let expected = row.checked_mul(height_bytes)?;
    let raw = image.as_raw();
    if raw.len() != expected || expected > MAX_NATIVE_RGBA_BYTES {
        return None;
    }

    let mut filtered = zeroed(expected.checked_add(height_bytes)?)?;
    for (line, output) in raw
        .chunks_exact(row)
        .zip(filtered.chunks_exact_mut(row + 1))
    {
        // The extra zero byte per scanline is PNG's NoFilter marker.
        output[1..].copy_from_slice(line);
    }
    let compressed = Compressor::new()?.compress(&filtered)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(compressed.len().checked_add(1024)?)
        .ok()?;
    {
        // png 0.16 writes IEND from Drop, so finish in memory before fallible file I/O.
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::RGBA);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_chunk(png::chunk::IDAT, &compressed).ok()?;
    }
    Some(output)
}

fn zeroed(len: usize) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    output.try_reserve_exact(len).ok()?;
    output.resize(len, 0);
    Some(output)
}

struct Compressor(NonNull<libdeflate_compressor>);

impl Compressor {
    fn new() -> Option<Self> {
        NonNull::new(unsafe { libdeflate_alloc_compressor(1) }).map(Self)
    }

    fn compress(&self, input: &[u8]) -> Option<Vec<u8>> {
        let bound = unsafe { libdeflate_zlib_compress_bound(self.0.as_ptr(), input.len()) };
        if bound == 0 {
            return None;
        }
        let mut output = zeroed(bound)?;
        // Both buffers remain live, and the writable length is exactly output.len().
        let written = unsafe {
            libdeflate_zlib_compress(
                self.0.as_ptr(),
                input.as_ptr().cast(),
                input.len(),
                output.as_mut_ptr().cast(),
                output.len(),
            )
        };
        if written == 0 || written > output.len() {
            return None;
        }
        output.truncate(written);
        Some(output)
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        unsafe { libdeflate_free_compressor(self.0.as_ptr()) };
    }
}
