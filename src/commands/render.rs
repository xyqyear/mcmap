// Render command — converts .mca region files to PNG overhead maps.
//
// Dispatch is implicit: `anvil::palette::load` detects the palette format
// and returns an `AnyPalette` variant; for each region we build the matching
// engine (modern / legacy) and feed it to the shared `render_region`
// pipeline. One code path, one set of bounds / parallelism / I/O logic.

use clap::Args;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

use super::util::{Rectangle, auto_size, parse_region_filename};
use crate::anvil::legacy::LegacyTopShadeRenderer;
use crate::anvil::{
    AnyPalette, CCoord, HeightMode, RCoord, RegionFileLoader, RegionMap, TopShadeRenderer,
    palette, region::RegionLoader, render_region,
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

/// Render one region using whichever engine the palette demands.
fn render_one(
    x: RCoord,
    z: RCoord,
    loader: &dyn RegionLoader,
    pal: &AnyPalette,
    height_mode: HeightMode,
) -> Result<Option<RegionMap>> {
    match pal {
        AnyPalette::Modern(p) => {
            let engine = TopShadeRenderer::new(p, height_mode);
            Ok(render_region(x, z, loader, &engine)?)
        }
        AnyPalette::Legacy(p, format) => {
            let engine = LegacyTopShadeRenderer::new(p, *format);
            Ok(render_region(x, z, loader, &engine)?)
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

    let height_mode = if args.calculate_heights {
        info!("Height mode: Calculate");
        HeightMode::Calculate
    } else {
        info!("Height mode: Trust heightmap");
        HeightMode::Trust
    };

    if !args.region.exists() {
        error!("Region path does not exist: {}", args.region.display());
        return Err(format!("Region path not found: {}", args.region.display()).into());
    }

    let (region_dir, coords) = if args.region.is_file() {
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
    let pal = palette::load(&args.palette)?;
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

    let Rectangle { xmin, xmax, zmin, zmax } = bounds;
    let x_range = xmin..xmax;
    let z_range = zmin..zmax;

    let region_len: usize = 32 * 16; // 32 chunks per region, 16 blocks per chunk

    info!("Rendering regions...");
    let render_start = std::time::Instant::now();
    let region_maps: Vec<_> = coords
        .into_par_iter()
        .filter_map(|(x, z)| {
            let loader = RegionFileLoader::new(region_dir.clone());

            if x < x_range.end && x >= x_range.start && z < z_range.end && z >= z_range.start {
                match render_one(x, z, &loader, &pal, height_mode) {
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

    if output_to_stdout {
        info!("Writing image to stdout");
        super::save_png(img, "-")?;
        info!("Image written to stdout successfully");
    } else {
        info!("Saving image to: {}", args.output);
        super::save_png(img, &args.output)?;
        info!("Done! Map saved successfully.");
    }
    info!("⏱ Total time: {:?}", total_start.elapsed());

    Ok(())
}
