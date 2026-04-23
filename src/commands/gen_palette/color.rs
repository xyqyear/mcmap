use fastanvil::Rgba;

/// Averages a raw RGBA image into a single color.
/// RGB is averaged only over pixels with alpha > 0 — this prevents sparse
/// textures (vines, fences, crops, fire) from being washed toward black by
/// their transparent background. Alpha is averaged over all pixels, so
/// coverage is preserved in the output alpha channel.
/// Uses quadratic mean (RMS) for perceptually better mixing.
pub fn avg_colour(rgba_data: &[u8]) -> Rgba {
    let mut rgb = [0f64; 3];
    let mut alpha_sq = 0f64;
    let mut total = 0usize;
    let mut opaque = 0usize;

    for p in rgba_data.chunks(4) {
        if p.len() < 4 {
            continue;
        }
        total += 1;
        alpha_sq += (p[3] as u64 * p[3] as u64) as f64;
        if p[3] > 0 {
            rgb[0] += (p[0] as u64 * p[0] as u64) as f64;
            rgb[1] += (p[1] as u64 * p[1] as u64) as f64;
            rgb[2] += (p[2] as u64 * p[2] as u64) as f64;
            opaque += 1;
        }
    }

    if total == 0 || opaque == 0 {
        return [0, 0, 0, 0];
    }

    [
        (rgb[0] / opaque as f64).sqrt() as u8,
        (rgb[1] / opaque as f64).sqrt() as u8,
        (rgb[2] / opaque as f64).sqrt() as u8,
        (alpha_sq / total as f64).sqrt() as u8,
    ]
}
