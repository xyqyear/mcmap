// Heightmap command - renders height-based color maps from MCA files

use clap::Args;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

use crate::anvil::{
    CCoord, HeightMode, RCoord, RegionFileLoader, Rgba,
    chunk::{Chunk, ChunkData},
    region::RegionLoader,
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

/// A single color mapping point: height -> RGBA
#[derive(Debug, Clone)]
struct ColorPoint {
    height: isize,
    color: Rgba,
}

#[derive(Debug)]
struct Rectangle {
    xmin: RCoord,
    xmax: RCoord,
    zmin: RCoord,
    zmax: RCoord,
}

/// Parse color mapping from JSON string
/// Expected format: [[-64,0,0,0,255],[0,0,0,255,255],[64,0,255,0,255],...]
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

    // Sort by height
    points.sort_by_key(|p| p.height);

    if points.is_empty() {
        return Err("Color mapping must have at least one point".into());
    }

    Ok(points)
}

/// Get default color mapping
/// -64: black, 0: blue, 128: green, 255: red
fn default_color_mapping() -> Vec<ColorPoint> {
    vec![
        ColorPoint {
            height: -64,
            color: [0, 0, 0, 255],
        },
        ColorPoint {
            height: 0,
            color: [0, 0, 255, 255],
        },
        ColorPoint {
            height: 128,
            color: [0, 255, 0, 255],
        },
        ColorPoint {
            height: 255,
            color: [255, 0, 0, 255],
        },
    ]
}

/// Convert height to color using custom color mapping with linear interpolation
fn height_to_color(height: isize, color_map: &[ColorPoint]) -> Rgba {
    // Handle edge cases
    if color_map.is_empty() {
        return [128, 128, 128, 255]; // Gray fallback
    }

    if height <= color_map[0].height {
        return color_map[0].color;
    }

    if height >= color_map[color_map.len() - 1].height {
        return color_map[color_map.len() - 1].color;
    }

    // Find the two points to interpolate between
    for i in 0..color_map.len() - 1 {
        let p1 = &color_map[i];
        let p2 = &color_map[i + 1];

        if height >= p1.height && height <= p2.height {
            // Linear interpolation
            let t = (height - p1.height) as f32 / (p2.height - p1.height) as f32;

            return [
                (p1.color[0] as f32 * (1.0 - t) + p2.color[0] as f32 * t) as u8,
                (p1.color[1] as f32 * (1.0 - t) + p2.color[1] as f32 * t) as u8,
                (p1.color[2] as f32 * (1.0 - t) + p2.color[2] as f32 * t) as u8,
                (p1.color[3] as f32 * (1.0 - t) + p2.color[3] as f32 * t) as u8,
            ];
        }
    }

    // Fallback (shouldn't reach here)
    color_map[color_map.len() - 1].color
}

/// Render a single chunk to heightmap colors
fn render_chunk_heightmap(
    chunk: &ChunkData,
    height_mode: HeightMode,
    color_map: &[ColorPoint],
) -> [Rgba; 16 * 16] {
    let mut data = [[0, 0, 0, 0]; 16 * 16];

    for z in 0..16 {
        for x in 0..16 {
            let air_height = chunk.surface_height(x, z, height_mode);
            let block_height = air_height - 1;

            // Check if there's actually a non-air block at the surface
            let has_block = match chunk {
                ChunkData::Post13(post13) => {
                    // For Post-1.13, use fastanvil's block method
                    if let Some(block) = fastanvil::Chunk::block(post13.inner(), x, block_height, z)
                    {
                        block.name() != "minecraft:air" && block.name() != "minecraft:cave_air"
                    } else {
                        false
                    }
                }
                ChunkData::Pre13(_) => {
                    // For Pre-1.13, check if block() returns None or is air
                    if let Some(block) = chunk.block(x, block_height, z) {
                        block.name != "minecraft:air"
                    } else {
                        false
                    }
                }
            };

            if !has_block {
                // Empty column - transparent
                data[z * 16 + x] = [0, 0, 0, 0];
                continue;
            }

            // Direct height to color mapping
            let colour = height_to_color(block_height, color_map);

            data[z * 16 + x] = colour;
        }
    }

    data
}

/// Map of a rendered region
struct RegionMap {
    pub x: RCoord,
    pub z: RCoord,
    data: Vec<Rgba>,
}

impl RegionMap {
    fn new(x: RCoord, z: RCoord, fill: Rgba) -> Self {
        let len = 32 * 16 * 32 * 16;
        Self {
            x,
            z,
            data: vec![fill; len],
        }
    }

    fn chunk(&self, x: CCoord, z: CCoord) -> &[Rgba] {
        let len = 16 * 16;
        let begin = (z.0 * 32 + x.0) as usize * len;
        &self.data[begin..begin + len]
    }

    fn chunk_mut(&mut self, x: CCoord, z: CCoord) -> &mut [Rgba] {
        let len = 16 * 16;
        let begin = (z.0 * 32 + x.0) as usize * len;
        &mut self.data[begin..begin + len]
    }
}

/// Render a region to a heightmap
fn render_region_heightmap(
    x: RCoord,
    z: RCoord,
    loader: &RegionFileLoader,
    height_mode: HeightMode,
    color_map: &[ColorPoint],
) -> Result<Option<RegionMap>> {
    let mut map = RegionMap::new(x, z, [0u8; 4]);

    let mut region = match loader.region(x, z)? {
        Some(r) => r,
        None => return Ok(None),
    };

    for z in 0usize..32 {
        for x in 0usize..32 {
            let data = map.chunk_mut(CCoord(x as isize), CCoord(z as isize));

            let chunk_data = region
                .read_chunk(x, z)
                .map_err(|e| format!("Failed to read chunk: {}", e))?;

            let chunk_data = match chunk_data {
                Some(data) => data,
                None => continue,
            };

            let chunk = ChunkData::from_bytes(&chunk_data)?;

            let rendered = render_chunk_heightmap(&chunk, height_mode, color_map);
            data.copy_from_slice(&rendered);
        }
    }

    Ok(Some(map))
}

fn parse_region_coords(filename: &str) -> Option<(RCoord, RCoord)> {
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() >= 4 && parts[0] == "r" {
        let x = parts[1].parse::<isize>().ok()?;
        let z = parts[2].parse::<isize>().ok()?;
        return Some((RCoord(x), RCoord(z)));
    }
    None
}

fn get_region_bounds(region_dir: &Path) -> Option<Rectangle> {
    let entries = std::fs::read_dir(region_dir).ok()?;
    let mut bounds = Rectangle {
        xmin: RCoord(isize::MAX),
        xmax: RCoord(isize::MIN),
        zmin: RCoord(isize::MAX),
        zmax: RCoord(isize::MIN),
    };

    for entry in entries.flatten() {
        if let Some(filename) = entry.file_name().to_str() {
            if let Some((x, z)) = parse_region_coords(filename) {
                bounds.xmin = RCoord(bounds.xmin.0.min(x.0));
                bounds.xmax = RCoord(bounds.xmax.0.max(x.0));
                bounds.zmin = RCoord(bounds.zmin.0.min(z.0));
                bounds.zmax = RCoord(bounds.zmax.0.max(z.0));
            }
        }
    }

    bounds.xmax = RCoord(bounds.xmax.0 + 1);
    bounds.zmax = RCoord(bounds.zmax.0 + 1);

    Some(bounds)
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

    // Parse or use default color mapping
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

    // Determine if input is a file or directory
    let (region_dir, coords) = if args.region.is_file() {
        // Single region file
        let filename = args
            .region
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename")?;

        let (x, z) = parse_region_coords(filename)
            .ok_or("Invalid region filename format. Expected: r.x.z.mca")?;

        info!(
            "Rendering single region file: {} at coordinates ({}, {})",
            filename, x.0, z.0
        );

        let parent = args.region.parent().ok_or("Cannot get parent directory")?;

        (parent.to_path_buf(), vec![(x, z)])
    } else {
        // Region directory
        let bounds = get_region_bounds(&args.region).ok_or("Failed to determine region bounds")?;

        info!("Bounds: {:?}", bounds);

        let mut coords = Vec::new();
        for x in bounds.xmin.0..bounds.xmax.0 {
            for z in bounds.zmin.0..bounds.zmax.0 {
                coords.push((RCoord(x), RCoord(z)));
            }
        }

        (args.region.clone(), coords)
    };

    // Calculate image dimensions
    let x_range = coords.iter().map(|(x, _)| *x).collect::<Vec<_>>();
    let z_range = coords.iter().map(|(_, z)| *z).collect::<Vec<_>>();
    let x_min = x_range.iter().min().unwrap();
    let x_max = RCoord(x_range.iter().max().unwrap().0 + 1);
    let z_min = z_range.iter().min().unwrap();
    let z_max = RCoord(z_range.iter().max().unwrap().0 + 1);

    info!("Rendering regions...");
    let render_start = std::time::Instant::now();
    let region_maps: Vec<_> = coords
        .into_par_iter()
        .filter_map(|(x, z)| {
            let loader = RegionFileLoader::new(region_dir.clone());
            let color_map = color_map.clone();

            if x < x_max && x >= *x_min && z < z_max && z >= *z_min {
                let map = render_region_heightmap(x, z, &loader, height_mode, &color_map);
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
                        error!("Error rendering r.{}.{}.mca: {}", x.0, z.0, e);
                        None
                    }
                }
            } else {
                None
            }
        })
        .collect();

    let render_time = render_start.elapsed();
    info!("⏱ Region rendering took: {:?}", render_time);

    info!("{} regions processed successfully", region_maps.len());

    // Assemble final image
    let width = ((x_max.0 - x_min.0) * 512) as u32;
    let height = ((z_max.0 - z_min.0) * 512) as u32;

    info!("Creating output image: {}x{} pixels", width, height);

    let assembly_start = std::time::Instant::now();
    info!("Assembling final image...");

    let mut img = image::RgbaImage::new(width, height);

    for map in region_maps {
        let offset_x = ((map.x.0 - x_min.0) * 512) as u32;
        let offset_z = ((map.z.0 - z_min.0) * 512) as u32;

        for chunk_z in 0..32 {
            for chunk_x in 0..32 {
                let chunk_data = map.chunk(CCoord(chunk_x), CCoord(chunk_z));
                for local_z in 0..16 {
                    for local_x in 0..16 {
                        let pixel = chunk_data[local_z * 16 + local_x];
                        let img_x = offset_x + (chunk_x as u32 * 16) + local_x as u32;
                        let img_z = offset_z + (chunk_z as u32 * 16) + local_z as u32;
                        img.put_pixel(img_x, img_z, image::Rgba(pixel));
                    }
                }
            }
        }
    }

    let assembly_time = assembly_start.elapsed();
    info!("⏱ Image assembly took: {:?}", assembly_time);

    // Save or output image as PNG
    if output_to_stdout {
        crate::commands::save_png(img, "-")?;
        info!("Done! Map written to stdout.");
    } else {
        info!("Saving image to: {}", args.output);
        crate::commands::save_png(img, &args.output)?;
        info!("Done! Map saved successfully.");
    }

    let total_time = total_start.elapsed();
    info!("⏱ Total time: {:?}", total_time);

    Ok(())
}
