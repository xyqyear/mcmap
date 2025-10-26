use clap::Parser;
use env_logger::Env;
use fastanvil::{render_region, CCoord, HeightMode, RCoord, RegionFileLoader, RegionLoader, RenderedPalette, Rgba, TopShadeRenderer};
use flate2::read::GzDecoder;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser, Debug)]
#[command(author, version, about = "Minecraft map renderer - converts MCA files to PNG overhead views", long_about = None)]
struct Args {
    /// Path to a region folder or a single .mca file
    #[arg(short, long)]
    region: PathBuf,

    /// Output PNG file path
    #[arg(short, long, default_value = "map.png")]
    output: PathBuf,

    /// Path to the palette.tar.gz file
    #[arg(short, long)]
    palette: PathBuf,

    /// Calculate heights instead of trusting heightmap data
    #[arg(long, default_value_t = false)]
    calculate_heights: bool,
}

#[derive(Debug)]
struct Rectangle {
    xmin: RCoord,
    xmax: RCoord,
    zmin: RCoord,
    zmax: RCoord,
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

fn get_palette(path: &Path) -> Result<RenderedPalette> {
    info!("Loading palette from: {}", path.display());

    let f = std::fs::File::open(path)?;
    let f = GzDecoder::new(f);
    let mut ar = tar::Archive::new(f);
    let mut grass = Err("no grass colour map");
    let mut foliage = Err("no foliage colour map");
    let mut blockstates = Err("no blockstate palette");

    for file in ar.entries()? {
        let mut file = file?;
        match file.path()?.to_str().ok_or("invalid path in TAR")? {
            "grass-colourmap.png" => {
                use std::io::Read;
                let mut buf = vec![];
                file.read_to_end(&mut buf)?;

                grass = Ok(
                    image::load_from_memory_with_format(&buf, image::ImageFormat::Png)?
                        .into_rgba8(),
                );
            }
            "foliage-colourmap.png" => {
                use std::io::Read;
                let mut buf = vec![];
                file.read_to_end(&mut buf)?;

                foliage = Ok(
                    image::load_from_memory_with_format(&buf, image::ImageFormat::Png)?
                        .into_rgba8(),
                );
            }
            "blockstates.json" => {
                let json: std::collections::HashMap<String, Rgba> =
                    serde_json::from_reader(file)?;
                blockstates = Ok(json);
            }
            _ => {}
        }
    }

    let p = RenderedPalette {
        blockstates: blockstates?,
        grass: grass?,
        foliage: foliage?,
    };

    info!("Palette loaded successfully");
    Ok(p)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let args = Args::parse();

    info!("Starting Minecraft map renderer");
    info!("Region path: {}", args.region.display());
    info!("Output: {}", args.output.display());

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
        let filename = args.region
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid file name")?;

        let (x, z) = parse_region_filename(filename)
            .ok_or_else(|| format!("Invalid region file name: {}", filename))?;

        info!("Rendering single region file: {} at coordinates ({}, {})", filename, x.0, z.0);

        let parent = args.region
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
        error!("Region path is neither a file nor a directory: {}", args.region.display());
        return Err(format!("Invalid region path: {}", args.region.display()).into());
    };

    let bounds = auto_size(&coords).ok_or("Failed to calculate bounds")?;
    info!("Bounds: {:?}", bounds);

    let x_range = bounds.xmin..bounds.xmax;
    let z_range = bounds.zmin..bounds.zmax;

    let region_len: usize = 32 * 16; // 32 chunks per region, 16 blocks per chunk

    let pal = get_palette(&args.palette)?;

    info!("Rendering regions...");
    let region_maps: Vec<_> = coords
        .into_par_iter()
        .filter_map(|(x, z)| {
            let loader = RegionFileLoader::new(region_dir.clone());

            if x < x_range.end && x >= x_range.start && z < z_range.end && z >= z_range.start {
                let drawer = TopShadeRenderer::new(&pal, height_mode);
                let map = render_region(x, z, &loader, drawer);
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

    info!("Saving image to: {}", args.output.display());
    img.save(&args.output)?;
    info!("Done! Map saved successfully.");

    Ok(())
}
