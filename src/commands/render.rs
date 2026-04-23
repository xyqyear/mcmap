// Render command — converts MCA files to PNG overhead maps.
//
// Auto-detects the palette format and dispatches between the two chunk
// codecs:
//
//   - Modern (1.13+): flat `{"namespace:name": [r,g,b,a]}` palette, parsed by
//     `fastanvil`.
//   - Legacy (1.7.10, optionally with NotEnoughIDs): wrapped
//     `{"format":"1.7.10","blocks":{"id|meta":[...]}}` palette, parsed by
//     the `anvil::legacy` module.
//
// Dispatch is at the per-chunk level so a world where only some sections
// use legacy storage still works — though in practice a palette is tied to
// a world and all its chunks are the same era.

use clap::Args;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

use crate::anvil::legacy::{LegacyPalette, LegacyTopShadeRenderer, render_legacy_region};
use crate::anvil::legacy::palette::{PaletteFormat, detect_palette_format};
use crate::anvil::{
    CCoord, HeightMode, RCoord, RegionFileLoader, RegionMap, RenderedPalette, Rgba,
    TopShadeRenderer, render_region,
};

const REGION_PX: u32 = 32 * 16; // 512 px per region side

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct RenderArgs {
    /// Path to a region folder or a single .mca file
    #[arg(short, long)]
    region: PathBuf,

    /// Output PNG file path (use '-' for stdout)
    #[arg(short, long, default_value = "map.png")]
    output: String,

    /// Path to the palette.json file
    #[arg(short, long)]
    palette: PathBuf,

    /// Calculate heights instead of trusting heightmap data (modern only)
    #[arg(long, default_value_t = false)]
    calculate_heights: bool,

    /// Save each region as its own PNG inside --output (treated as a directory).
    /// File names mirror the region's .mca name (e.g. r.0.0.mca -> r.0.0.png).
    #[arg(long, default_value_t = false)]
    split: bool,

    /// Copy each source .mca file's modification time onto its generated PNG.
    /// Only valid with --split (requires a 1:1 region-to-file mapping).
    #[arg(long, default_value_t = false, requires = "split")]
    preserve_mtime: bool,
}

#[derive(Debug)]
struct Rectangle {
    xmin: RCoord,
    xmax: RCoord,
    zmin: RCoord,
    zmax: RCoord,
}

/// Opaque palette holder — lets us carry both variants through the render
/// pipeline without loading both unconditionally.
enum AnyPalette {
    Modern(RenderedPalette),
    Legacy(LegacyPalette),
}

fn parse_region_filename(filename: &str) -> Option<(RCoord, RCoord)> {
    // Expected format: r.X.Z.mca
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        return None;
    }

    let x: isize = parts[1].parse().ok()?;
    let z: isize = parts[2].parse().ok()?;
    Some((RCoord(x), RCoord(z)))
}

fn auto_size(coords: &[(RCoord, RCoord)]) -> Option<Rectangle> {
    if coords.is_empty() {
        return None;
    }

    let mut bounds = Rectangle {
        xmin: RCoord(isize::MAX),
        zmin: RCoord(isize::MAX),
        xmax: RCoord(isize::MIN),
        zmax: RCoord(isize::MIN),
    };

    for coord in coords {
        bounds.xmin = std::cmp::min(bounds.xmin, coord.0);
        bounds.xmax = std::cmp::max(bounds.xmax, coord.0);
        bounds.zmin = std::cmp::min(bounds.zmin, coord.1);
        bounds.zmax = std::cmp::max(bounds.zmax, coord.1);
    }

    // Add 1 to max bounds to make the range inclusive
    bounds.xmax = RCoord(bounds.xmax.0 + 1);
    bounds.zmax = RCoord(bounds.zmax.0 + 1);

    Some(bounds)
}

fn load_palette(path: &Path) -> Result<AnyPalette> {
    info!("Loading palette from: {}", path.display());
    match detect_palette_format(path)? {
        PaletteFormat::Modern => {
            let bytes = std::fs::read(path)?;
            let blockstates: std::collections::HashMap<String, Rgba> =
                serde_json::from_slice(&bytes)?;
            info!(
                "Palette loaded: {} block states (modern / 1.13+)",
                blockstates.len()
            );
            Ok(AnyPalette::Modern(RenderedPalette::new(blockstates)))
        }
        PaletteFormat::Legacy => {
            let pal = LegacyPalette::load(path)?;
            info!("Palette loaded: {} entries (legacy / 1.7.10)", pal.len());
            Ok(AnyPalette::Legacy(pal))
        }
    }
}

/// Copy the modification time of `src` onto `dst`. Used so caches keyed on
/// mtime can treat the rendered PNG as being "as fresh as" the source region.
fn copy_mtime(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mtime = std::fs::metadata(src)?.modified()?;
    let dst_file = std::fs::File::options().write(true).open(dst)?;
    dst_file.set_modified(mtime)
}

/// Copy a rendered `RegionMap` into a freshly allocated 512×512 `RgbaImage`.
fn region_to_image(map: &RegionMap) -> image::RgbaImage {
    let mut img = image::RgbaImage::new(REGION_PX, REGION_PX);
    for xc in 0..32 {
        for zc in 0..32 {
            let chunk = map.chunk(CCoord(xc), CCoord(zc));
            for z in 0..16 {
                for x in 0..16 {
                    let pixel = chunk[z * 16 + x];
                    let px = (xc as u32) * 16 + x as u32;
                    let pz = (zc as u32) * 16 + z as u32;
                    img.put_pixel(px, pz, image::Rgba(pixel));
                }
            }
        }
    }
    img
}

/// Render one region using whichever path the palette requires.
fn render_one(
    x: RCoord,
    z: RCoord,
    loader: &dyn crate::anvil::region::RegionLoader,
    pal: &AnyPalette,
    height_mode: HeightMode,
) -> Result<Option<RegionMap>> {
    match pal {
        AnyPalette::Modern(p) => {
            let drawer = TopShadeRenderer::new(p, height_mode);
            Ok(render_region(x, z, loader, drawer)?)
        }
        AnyPalette::Legacy(p) => {
            let drawer = LegacyTopShadeRenderer::new(p);
            Ok(render_legacy_region(x, z, loader, drawer)?)
        }
    }
}

pub fn execute(args: RenderArgs) -> Result<()> {
    let total_start = std::time::Instant::now();

    info!("Starting Minecraft map renderer");
    info!("Region path: {}", args.region.display());

    let output_to_stdout = args.output == "-";
    if args.split && output_to_stdout {
        error!("--split cannot be combined with stdout output");
        return Err("--split cannot be combined with stdout output".into());
    }

    if args.split {
        info!("Output directory: {}", args.output);
    } else if output_to_stdout {
        info!("Output: stdout");
    } else {
        info!("Output: {}", args.output);
    }

    let height_mode = match args.calculate_heights {
        true => {
            info!("Height mode: Calculate");
            HeightMode::Calculate
        }
        false => {
            info!("Height mode: Trust heightmap");
            HeightMode::Trust
        }
    };

    if !args.region.exists() {
        error!("Region path does not exist: {}", args.region.display());
        return Err(format!("Region path not found: {}", args.region.display()).into());
    }

    // Determine if input is a file or directory
    let (region_dir, coords) = if args.region.is_file() {
        // Single region file
        let filename = args
            .region
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid file name")?;

        let (x, z) = parse_region_filename(filename)
            .ok_or_else(|| format!("Invalid region file name: {}", filename))?;

        info!(
            "Rendering single region file: {} at coordinates ({}, {})",
            filename, x.0, z.0
        );

        let parent = args
            .region
            .parent()
            .ok_or("Could not get parent directory")?
            .to_path_buf();

        (parent, vec![(x, z)])
    } else if args.region.is_dir() {
        // Region directory
        info!("Loading regions from directory: {}", args.region.display());
        let loader = RegionFileLoader::new(args.region.clone());
        let coords = loader.list()?;
        info!("Found {} region files", coords.len());

        if coords.is_empty() {
            error!("No region files found in {}", args.region.display());
            return Err("No region files found".into());
        }

        (args.region.clone(), coords)
    } else {
        error!(
            "Region path is neither a file nor a directory: {}",
            args.region.display()
        );
        return Err(format!("Invalid region path: {}", args.region.display()).into());
    };

    let palette_start = std::time::Instant::now();
    let pal = load_palette(&args.palette)?;
    info!("⏱ Palette loading took: {:?}", palette_start.elapsed());

    if args.split {
        let out_dir = PathBuf::from(&args.output);
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("Failed to create output directory {}: {}", args.output, e))?;

        info!("Rendering regions (split mode)...");
        let render_start = std::time::Instant::now();
        let saved: usize = coords
            .into_par_iter()
            .map(|(x, z)| {
                let loader = RegionFileLoader::new(region_dir.clone());
                let src_path = region_dir.join(format!("r.{}.{}.mca", x.0, z.0));
                match render_one(x, z, &loader, &pal, height_mode) {
                    Ok(Some(map)) => {
                        let img = region_to_image(&map);
                        let path = out_dir.join(format!("r.{}.{}.png", x.0, z.0));
                        match img.save(&path) {
                            Ok(()) => {
                                if args.preserve_mtime {
                                    if let Err(e) = copy_mtime(&src_path, &path) {
                                        warn!(
                                            "Saved {} but failed to copy mtime from {}: {}",
                                            path.display(),
                                            src_path.display(),
                                            e
                                        );
                                    }
                                }
                                info!("Saved {}", path.display());
                                1
                            }
                            Err(e) => {
                                error!("Failed to save {}: {}", path.display(), e);
                                0
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("Missing r.{}.{}.mca", x.0, z.0);
                        0
                    }
                    Err(e) => {
                        error!("Error processing r.{}.{}.mca: {}", x.0, z.0, e);
                        0
                    }
                }
            })
            .sum();
        info!("⏱ Region rendering took: {:?}", render_start.elapsed());
        info!("{} regions saved to {}", saved, out_dir.display());
        info!("⏱ Total time: {:?}", total_start.elapsed());
        return Ok(());
    }

    let bounds = auto_size(&coords).ok_or("Failed to calculate bounds")?;
    info!("Bounds: {:?}", bounds);

    let x_range = bounds.xmin..bounds.xmax;
    let z_range = bounds.zmin..bounds.zmax;

    let region_len: usize = 32 * 16; // 32 chunks per region, 16 blocks per chunk

    info!("Rendering regions...");
    let render_start = std::time::Instant::now();
    let region_maps: Vec<_> = coords
        .into_par_iter()
        .filter_map(|(x, z)| {
            let loader = RegionFileLoader::new(region_dir.clone());

            if x < x_range.end && x >= x_range.start && z < z_range.end && z >= z_range.start {
                let map = render_one(x, z, &loader, &pal, height_mode);
                match map {
                    Ok(Some(map)) => {
                        info!("Processed r.{}.{}.mca", x.0, z.0);
                        Some(map)
                    }
                    Ok(None) => {
                        warn!("Missing r.{}.{}.mca", x.0, z.0);
                        None
                    }
                    Err(e) => {
                        error!("Error processing r.{}.{}.mca: {}", x.0, z.0, e);
                        None
                    }
                }
            } else {
                None
            }
        })
        .collect();
    info!("⏱ Region rendering took: {:?}", render_start.elapsed());

    info!("{} regions processed successfully", region_maps.len());

    let dx = (x_range.end.0 - x_range.start.0) as usize;
    let dz = (z_range.end.0 - z_range.start.0) as usize;

    info!(
        "Creating output image: {}x{} pixels",
        dx * region_len,
        dz * region_len
    );
    let mut img = image::ImageBuffer::new((dx * region_len) as u32, (dz * region_len) as u32);

    info!("Assembling final image...");
    let assemble_start = std::time::Instant::now();
    for map in region_maps {
        let xrp = map.x.0 - x_range.start.0;
        let zrp = map.z.0 - z_range.start.0;

        for xc in 0..32 {
            for zc in 0..32 {
                let chunk = map.chunk(CCoord(xc), CCoord(zc));
                let xcp = xrp * 32 + xc;
                let zcp = zrp * 32 + zc;

                for z in 0..16 {
                    for x in 0..16 {
                        let pixel = chunk[z * 16 + x];
                        let x = xcp * 16 + x as isize;
                        let z = zcp * 16 + z as isize;
                        img.put_pixel(x as u32, z as u32, image::Rgba(pixel))
                    }
                }
            }
        }
    }
    info!("⏱ Image assembly took: {:?}", assemble_start.elapsed());

    // Save image to file or stdout as PNG
    if output_to_stdout {
        info!("Writing image to stdout");
        crate::commands::save_png(img, "-")?;
        info!("Image written to stdout successfully");
    } else {
        info!("Saving image to: {}", args.output);
        crate::commands::save_png(img, &args.output)?;
        info!("Done! Map saved successfully.");
    }
    info!("⏱ Total time: {:?}", total_start.elapsed());

    Ok(())
}
