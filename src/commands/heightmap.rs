// Heightmap command — renders height-based color maps from .mca files.
// 1.13+ only (no legacy support). Plugs into the same region pipeline as
// `render` via a purpose-built `HeightmapEngine`, so allocation, iteration,
// and parallel scheduling stay shared.

use clap::Args;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::PathBuf;

use super::util::{Rectangle, auto_size, parse_region_filename};
use crate::anvil::modern::ChunkData;
use crate::anvil::{
    CCoord, HeightMode, RegionFileLoader, RegionMap, RenderEngine, Rgba, render_region,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct HeightmapArgs {
    /// Path to a region folder or a single .mca file
    #[arg(short, long)]
    region: PathBuf,

    /// Output PNG file path (use '-' for stdout)
    #[arg(short, long, default_value = "heightmap.png")]
    output: String,

    /// Calculate heights instead of trusting heightmap data
    #[arg(long, default_value_t = false)]
    calculate_heights: bool,

    /// Custom color mapping as JSON array: [[-64,0,0,0,255],[0,0,0,255,255],[64,0,255,0,255],...]
    /// Format: [[height, r, g, b, a], ...]
    #[arg(long)]
    colors: Option<String>,
}

/// A single color mapping point: height -> RGBA.
#[derive(Debug, Clone)]
struct ColorPoint {
    height: isize,
    color: Rgba,
}

fn parse_color_mapping(json_str: &str) -> Result<Vec<ColorPoint>> {
    let data: Vec<Vec<i32>> = serde_json::from_str(json_str)?;
    let mut points = Vec::new();
    for entry in data {
        if entry.len() != 5 {
            return Err("Each color point must have 5 values: [height, r, g, b, a]".into());
        }
        points.push(ColorPoint {
            height: entry[0] as isize,
            color: [
                entry[1] as u8,
                entry[2] as u8,
                entry[3] as u8,
                entry[4] as u8,
            ],
        });
    }
    points.sort_by_key(|p| p.height);
    if points.is_empty() {
        return Err("Color mapping must have at least one point".into());
    }
    Ok(points)
}

/// Default: -64 black, 0 blue, 128 green, 255 red.
fn default_color_mapping() -> Vec<ColorPoint> {
    vec![
        ColorPoint { height: -64, color: [0, 0, 0, 255] },
        ColorPoint { height: 0, color: [0, 0, 255, 255] },
        ColorPoint { height: 128, color: [0, 255, 0, 255] },
        ColorPoint { height: 255, color: [255, 0, 0, 255] },
    ]
}

/// Linear interpolation over the color-point table. Clamps below/above the
/// first/last point; returns gray as a safety fallback when the table is
/// empty (which the parser already rejects, but belt-and-suspenders).
fn height_to_color(height: isize, color_map: &[ColorPoint]) -> Rgba {
    if color_map.is_empty() {
        return [128, 128, 128, 255];
    }
    if height <= color_map[0].height {
        return color_map[0].color;
    }
    if height >= color_map[color_map.len() - 1].height {
        return color_map[color_map.len() - 1].color;
    }
    for i in 0..color_map.len() - 1 {
        let p1 = &color_map[i];
        let p2 = &color_map[i + 1];
        if height >= p1.height && height <= p2.height {
            let t = (height - p1.height) as f32 / (p2.height - p1.height) as f32;
            return [
                (p1.color[0] as f32 * (1.0 - t) + p2.color[0] as f32 * t) as u8,
                (p1.color[1] as f32 * (1.0 - t) + p2.color[1] as f32 * t) as u8,
                (p1.color[2] as f32 * (1.0 - t) + p2.color[2] as f32 * t) as u8,
                (p1.color[3] as f32 * (1.0 - t) + p2.color[3] as f32 * t) as u8,
            ];
        }
    }
    color_map[color_map.len() - 1].color
}

/// Modern-only heightmap engine. Doesn't need the neighbour chunk for
/// shading, so the pipeline skips the above-region read.
struct HeightmapEngine<'a> {
    height_mode: HeightMode,
    color_map: &'a [ColorPoint],
}

impl<'a> RenderEngine for HeightmapEngine<'a> {
    type Chunk = ChunkData;
    const NEEDS_NORTH_CACHE: bool = false;

    fn decode(&self, bytes: &[u8]) -> std::result::Result<Option<Self::Chunk>, String> {
        ChunkData::from_bytes(bytes).map(Some)
    }

    fn render_chunk(&self, chunk: &ChunkData, _north: Option<&ChunkData>) -> [Rgba; 16 * 16] {
        let mut data = [[0, 0, 0, 0]; 16 * 16];
        for z in 0..16 {
            for x in 0..16 {
                let air_height =
                    fastanvil::Chunk::surface_height(chunk.inner(), x, z, self.height_mode);
                let block_height = air_height - 1;
                let has_block =
                    if let Some(block) = fastanvil::Chunk::block(chunk.inner(), x, block_height, z)
                    {
                        block.name() != "minecraft:air" && block.name() != "minecraft:cave_air"
                    } else {
                        false
                    };
                if !has_block {
                    data[z * 16 + x] = [0, 0, 0, 0];
                    continue;
                }
                data[z * 16 + x] = height_to_color(block_height, self.color_map);
            }
        }
        data
    }
}

pub fn execute(args: HeightmapArgs) -> Result<()> {
    let total_start = std::time::Instant::now();

    info!("Starting heightmap renderer");
    info!("Region path: {}", args.region.display());

    let output_to_stdout = args.output == "-";
    if output_to_stdout {
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

    let color_map = if let Some(ref colors_json) = args.colors {
        info!("Using custom color mapping");
        parse_color_mapping(colors_json)?
    } else {
        info!("Using default color mapping");
        default_color_mapping()
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
            .ok_or("Invalid filename")?;
        let (x, z) = parse_region_filename(filename)
            .ok_or("Invalid region filename format. Expected: r.x.z.mca")?;
        info!(
            "Rendering single region file: {} at coordinates ({}, {})",
            filename, x.0, z.0
        );
        let parent = args.region.parent().ok_or("Cannot get parent directory")?;
        (parent.to_path_buf(), vec![(x, z)])
    } else {
        let loader = RegionFileLoader::new(args.region.clone());
        let coords = loader.list()?;
        if coords.is_empty() {
            return Err(
                format!("No region files found in {}", args.region.display()).into()
            );
        }
        (args.region.clone(), coords)
    };

    let bounds = auto_size(&coords).ok_or("Failed to determine region bounds")?;
    info!("Bounds: {:?}", bounds);

    let Rectangle { xmin, xmax, zmin, zmax } = bounds;
    let x_range = xmin..xmax;
    let z_range = zmin..zmax;

    info!("Rendering regions...");
    let render_start = std::time::Instant::now();
    let engine = HeightmapEngine { height_mode, color_map: &color_map };
    let region_maps: Vec<RegionMap> = coords
        .into_par_iter()
        .filter_map(|(x, z)| {
            if x < x_range.end && x >= x_range.start && z < z_range.end && z >= z_range.start {
                let loader = RegionFileLoader::new(region_dir.clone());
                match render_region(x, z, &loader, &engine) {
                    Ok(Some(map)) => {
                        info!("Processed r.{}.{}.mca", x.0, z.0);
                        Some(map)
                    }
                    Ok(None) => {
                        warn!("Missing r.{}.{}.mca", x.0, z.0);
                        None
                    }
                    Err(e) => {
                        error!("Error rendering r.{}.{}.mca: {}", x.0, z.0, e);
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

    let width = ((x_range.end.0 - x_range.start.0) * 512) as u32;
    let height = ((z_range.end.0 - z_range.start.0) * 512) as u32;
    info!("Creating output image: {}x{} pixels", width, height);

    let assembly_start = std::time::Instant::now();
    info!("Assembling final image...");
    let mut img = image::RgbaImage::new(width, height);
    for map in region_maps {
        let offset_x = ((map.x.0 - x_range.start.0) * 512) as u32;
        let offset_z = ((map.z.0 - z_range.start.0) * 512) as u32;
        for chunk_z in 0..32 {
            for chunk_x in 0..32 {
                let chunk = map.chunk(CCoord(chunk_x), CCoord(chunk_z));
                for local_z in 0..16 {
                    for local_x in 0..16 {
                        let pixel = chunk[local_z * 16 + local_x];
                        let img_x = offset_x + (chunk_x as u32 * 16) + local_x as u32;
                        let img_z = offset_z + (chunk_z as u32 * 16) + local_z as u32;
                        img.put_pixel(img_x, img_z, image::Rgba(pixel));
                    }
                }
            }
        }
    }
    info!("⏱ Image assembly took: {:?}", assembly_start.elapsed());

    if output_to_stdout {
        super::save_png(img, "-")?;
        info!("Done! Map written to stdout.");
    } else {
        info!("Saving image to: {}", args.output);
        super::save_png(img, &args.output)?;
        info!("Done! Map saved successfully.");
    }
    info!("⏱ Total time: {:?}", total_start.elapsed());
    Ok(())
}
