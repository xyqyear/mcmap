// Render command — converts .mca region files to PNG overhead maps.
//
// Dispatch is implicit: `anvil::palette::load` detects the palette format
// and returns an `AnyPalette` variant; for each region we build the matching
// engine (modern / legacy) and feed it to the shared `render_region`
// pipeline. One code path, one set of bounds / parallelism / I/O logic.

use clap::Args;
use log::{error, info, warn};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::util::{Rectangle, auto_size, parse_region_filename};
use crate::anvil::legacy::LegacyTopShadeRenderer;
use crate::anvil::{
    AnyPalette, CCoord, HeightMode, RCoord, RegionFileLoader, RegionMap, TopShadeRenderer,
    palette, region::RegionLoader, render_region,
};
use crate::chown;
use crate::output::emit_if_json;

const REGION_PX: u32 = 32 * 16; // 512 px per region side

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct RenderArgs {
    /// Path to a region folder or .mca file. Repeat -r to combine multiple
    /// sources; duplicate region coordinates are deduplicated (last wins).
    #[arg(short, long, required = true)]
    region: Vec<PathBuf>,

    /// Output PNG file path
    #[arg(short, long, default_value = "map.png")]
    output: PathBuf,

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

    /// Number of worker threads for parallel region rendering.
    /// Defaults to the number of logical CPUs.
    #[arg(short = 'j', long, value_name = "N")]
    threads: Option<usize>,
}

#[derive(Serialize)]
struct PhaseEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    phase: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    elapsed_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bounds: Option<BoundsJson>,
}

#[derive(Serialize)]
struct BoundsJson {
    xmin: i64,
    xmax: i64,
    zmin: i64,
    zmax: i64,
}

#[derive(Serialize)]
struct RegionEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    x: i64,
    z: i64,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct ResultEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    mode: &'a str,
    regions_saved: usize,
    output: String,
    elapsed_ms: u128,
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

fn emit_phase(phase: &str) {
    emit_if_json(&PhaseEvent {
        ty: "progress",
        phase,
        elapsed_ms: None,
        count: None,
        bounds: None,
    });
}

pub fn execute(args: RenderArgs) -> Result<()> {
    let total_start = std::time::Instant::now();

    if let Some(n) = args.threads {
        if n == 0 {
            return Err("--threads must be >= 1".into());
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .map_err(|e| format!("Failed to configure thread pool: {}", e))?;
        info!("Worker threads: {}", n);
    }

    info!("Starting Minecraft map renderer");
    for path in &args.region {
        info!("Region path: {}", path.display());
    }

    if args.split {
        info!("Output directory: {}", args.output.display());
    } else {
        info!("Output: {}", args.output.display());
    }

    let height_mode = if args.calculate_heights {
        info!("Height mode: Calculate");
        HeightMode::Calculate
    } else {
        info!("Height mode: Trust heightmap");
        HeightMode::Trust
    };

    let mut entries_map: HashMap<(isize, isize), PathBuf> = HashMap::new();
    for path in &args.region {
        if !path.exists() {
            error!("Region path does not exist: {}", path.display());
            return Err(format!("Region path not found: {}", path.display()).into());
        }

        if path.is_file() {
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Invalid file name")?;

            let (x, z) = parse_region_filename(filename)
                .ok_or_else(|| format!("Invalid region file name: {}", filename))?;

            info!(
                "Including region file: {} at coordinates ({}, {})",
                filename, x.0, z.0
            );

            let parent = path
                .parent()
                .ok_or("Could not get parent directory")?
                .to_path_buf();

            entries_map.insert((x.0, z.0), parent);
        } else if path.is_dir() {
            info!("Loading regions from directory: {}", path.display());
            let loader = RegionFileLoader::new(path.clone());
            let dir_coords = loader.list()?;
            info!("Found {} region files in {}", dir_coords.len(), path.display());
            for (x, z) in dir_coords {
                entries_map.insert((x.0, z.0), path.clone());
            }
        } else {
            error!(
                "Region path is neither a file nor a directory: {}",
                path.display()
            );
            return Err(format!("Invalid region path: {}", path.display()).into());
        }
    }

    if entries_map.is_empty() {
        error!("No region files found across provided --region inputs");
        return Err("No region files found".into());
    }

    let entries: Vec<(PathBuf, RCoord, RCoord)> = entries_map
        .into_iter()
        .map(|((x, z), parent)| (parent, RCoord(x), RCoord(z)))
        .collect();
    let coords: Vec<(RCoord, RCoord)> = entries.iter().map(|(_, x, z)| (*x, *z)).collect();
    info!("Total unique regions: {}", entries.len());

    let palette_start = std::time::Instant::now();
    let pal = palette::load(&args.palette)?;
    let palette_elapsed = palette_start.elapsed();
    info!("⏱ Palette loading took: {:?}", palette_elapsed);
    emit_if_json(&PhaseEvent {
        ty: "progress",
        phase: "palette_loaded",
        elapsed_ms: Some(palette_elapsed.as_millis()),
        count: None,
        bounds: None,
    });

    if args.split {
        let out_dir = args.output.clone();
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            format!("Failed to create output directory {}: {}", out_dir.display(), e)
        })?;
        chown::apply(&out_dir).map_err(|e| {
            format!("Failed to chown output directory {}: {}", out_dir.display(), e)
        })?;

        emit_if_json(&PhaseEvent {
            ty: "progress",
            phase: "regions_listed",
            elapsed_ms: None,
            count: Some(entries.len()),
            bounds: None,
        });

        info!("Rendering regions (split mode)...");
        let render_start = std::time::Instant::now();
        let saved: usize = entries
            .into_par_iter()
            .map(|(parent, x, z)| {
                let loader = RegionFileLoader::new(parent.clone());
                let src_path = parent.join(format!("r.{}.{}.mca", x.0, z.0));
                match render_one(x, z, &loader, &pal, height_mode) {
                    Ok(Some(map)) => {
                        let img = region_to_image(&map);
                        let path = out_dir.join(format!("r.{}.{}.png", x.0, z.0));
                        let save_then_chown = img
                            .save(&path)
                            .map_err(|e| format!("save failed: {}", e))
                            .and_then(|()| {
                                chown::apply(&path)
                                    .map_err(|e| format!("chown failed: {}", e))
                            });
                        match save_then_chown {
                            Ok(()) => {
                                let mut warnings: Vec<String> = Vec::new();
                                if args.preserve_mtime {
                                    if let Err(e) = copy_mtime(&src_path, &path) {
                                        warn!(
                                            "Saved {} but failed to copy mtime from {}: {}",
                                            path.display(),
                                            src_path.display(),
                                            e
                                        );
                                        warnings.push(format!("mtime_copy_failed: {}", e));
                                    }
                                }
                                info!("Saved {}", path.display());
                                emit_if_json(&RegionEvent {
                                    ty: "region",
                                    x: x.0 as i64,
                                    z: z.0 as i64,
                                    status: "rendered",
                                    output: Some(path.display().to_string()),
                                    error: None,
                                    warnings,
                                });
                                1
                            }
                            Err(e) => {
                                error!("Failed to save {}: {}", path.display(), e);
                                emit_if_json(&RegionEvent {
                                    ty: "region",
                                    x: x.0 as i64,
                                    z: z.0 as i64,
                                    status: "error",
                                    output: None,
                                    error: Some(e),
                                    warnings: Vec::new(),
                                });
                                0
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("Missing r.{}.{}.mca", x.0, z.0);
                        emit_if_json(&RegionEvent {
                            ty: "region",
                            x: x.0 as i64,
                            z: z.0 as i64,
                            status: "missing",
                            output: None,
                            error: None,
                            warnings: Vec::new(),
                        });
                        0
                    }
                    Err(e) => {
                        error!("Error processing r.{}.{}.mca: {}", x.0, z.0, e);
                        emit_if_json(&RegionEvent {
                            ty: "region",
                            x: x.0 as i64,
                            z: z.0 as i64,
                            status: "error",
                            output: None,
                            error: Some(e.to_string()),
                            warnings: Vec::new(),
                        });
                        0
                    }
                }
            })
            .sum();
        info!("⏱ Region rendering took: {:?}", render_start.elapsed());
        info!("{} regions saved to {}", saved, out_dir.display());
        info!("⏱ Total time: {:?}", total_start.elapsed());
        emit_if_json(&ResultEvent {
            ty: "result",
            mode: "split",
            regions_saved: saved,
            output: out_dir.display().to_string(),
            elapsed_ms: total_start.elapsed().as_millis(),
        });
        return Ok(());
    }

    let bounds = auto_size(&coords).ok_or("Failed to calculate bounds")?;
    info!("Bounds: {:?}", bounds);

    let Rectangle { xmin, xmax, zmin, zmax } = bounds;
    emit_if_json(&PhaseEvent {
        ty: "progress",
        phase: "regions_listed",
        elapsed_ms: None,
        count: Some(coords.len()),
        bounds: Some(BoundsJson {
            xmin: xmin.0 as i64,
            xmax: xmax.0 as i64,
            zmin: zmin.0 as i64,
            zmax: zmax.0 as i64,
        }),
    });

    let x_range = xmin..xmax;
    let z_range = zmin..zmax;

    let region_len: usize = 32 * 16; // 32 chunks per region, 16 blocks per chunk

    info!("Rendering regions...");
    let render_start = std::time::Instant::now();
    let region_maps: Vec<_> = entries
        .into_par_iter()
        .filter_map(|(parent, x, z)| {
            let loader = RegionFileLoader::new(parent);

            if x < x_range.end && x >= x_range.start && z < z_range.end && z >= z_range.start {
                match render_one(x, z, &loader, &pal, height_mode) {
                    Ok(Some(map)) => {
                        info!("Processed r.{}.{}.mca", x.0, z.0);
                        emit_if_json(&RegionEvent {
                            ty: "region",
                            x: x.0 as i64,
                            z: z.0 as i64,
                            status: "rendered",
                            output: None,
                            error: None,
                            warnings: Vec::new(),
                        });
                        Some(map)
                    }
                    Ok(None) => {
                        warn!("Missing r.{}.{}.mca", x.0, z.0);
                        emit_if_json(&RegionEvent {
                            ty: "region",
                            x: x.0 as i64,
                            z: z.0 as i64,
                            status: "missing",
                            output: None,
                            error: None,
                            warnings: Vec::new(),
                        });
                        None
                    }
                    Err(e) => {
                        error!("Error processing r.{}.{}.mca: {}", x.0, z.0, e);
                        emit_if_json(&RegionEvent {
                            ty: "region",
                            x: x.0 as i64,
                            z: z.0 as i64,
                            status: "error",
                            output: None,
                            error: Some(e.to_string()),
                            warnings: Vec::new(),
                        });
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
    let region_count = region_maps.len();

    let dx = (x_range.end.0 - x_range.start.0) as usize;
    let dz = (z_range.end.0 - z_range.start.0) as usize;

    info!(
        "Creating output image: {}x{} pixels",
        dx * region_len,
        dz * region_len
    );
    let mut img = image::ImageBuffer::new((dx * region_len) as u32, (dz * region_len) as u32);

    emit_phase("assembling");
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

    info!("Saving image to: {}", args.output.display());
    img.save(&args.output)?;
    chown::apply(&args.output)
        .map_err(|e| format!("Failed to chown {}: {}", args.output.display(), e))?;
    info!("Done! Map saved successfully.");
    info!("⏱ Total time: {:?}", total_start.elapsed());

    emit_if_json(&ResultEvent {
        ty: "result",
        mode: "combined",
        regions_saved: region_count,
        output: args.output.display().to_string(),
        elapsed_ms: total_start.elapsed().as_millis(),
    });

    Ok(())
}
