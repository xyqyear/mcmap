// Analyze command - finds blocks not present in the palette

use clap::Args;
use log::{error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::anvil::region::RegionLoader;
use crate::anvil::{RCoord, RegionFileLoader, RenderedPalette, Rgba};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Path to a region folder or a single .mca file
    #[arg(short, long)]
    region: PathBuf,

    /// Path to the palette.tar.gz file
    #[arg(short, long)]
    palette: PathBuf,

    /// Show block counts (how many times each unknown block appears)
    #[arg(long, default_value_t = false)]
    show_counts: bool,
}

fn get_palette(path: &Path) -> Result<RenderedPalette> {
    info!("Loading palette from: {}", path.display());

    // Load the palette.json file directly
    let file = std::fs::File::open(path)?;
    let blockstates: std::collections::HashMap<String, Rgba> = serde_json::from_reader(file)?;

    info!(
        "Palette loaded successfully: {} block states",
        blockstates.len()
    );

    // Load idmap.json for Pre-1.13 block ID to name mapping
    let idmap_path = Path::new("idmap.json");
    let idmap: std::collections::HashMap<u16, String> = if idmap_path.exists() {
        let file = std::fs::File::open(idmap_path)?;
        let map: std::collections::HashMap<String, String> = serde_json::from_reader(file)?;
        // Convert string keys to u16
        map.into_iter()
            .filter_map(|(k, v)| k.parse::<u16>().ok().map(|id| (id, v)))
            .collect()
    } else {
        info!("idmap.json not found, using raw block IDs for Pre-1.13 blocks");
        std::collections::HashMap::new()
    };

    info!("Block ID map loaded: {} mappings", idmap.len());

    Ok(RenderedPalette { blockstates, idmap })
}

fn analyze_chunk_blocks(
    chunk_data: &[u8],
    blocks_found: &mut HashMap<String, usize>,
    palette: &RenderedPalette,
) -> Result<()> {
    use crate::anvil::chunk::ChunkData;

    // Parse chunk using our version-aware parser
    match ChunkData::from_bytes(chunk_data) {
        Ok(ChunkData::Post13(chunk)) => {
            // Post-1.13 chunk - use fastanvil to get block names
            let y_range = fastanvil::Chunk::y_range(chunk.inner());

            for y in y_range {
                for z in 0..16 {
                    for x in 0..16 {
                        if let Some(block) = fastanvil::Chunk::block(chunk.inner(), x, y, z) {
                            let block_name = block.name().to_string();
                            *blocks_found.entry(block_name).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        Ok(ChunkData::Pre13(chunk)) => {
            // Pre-1.13 chunk - use idmap to convert block IDs to names
            use crate::anvil::chunk::Chunk;
            let y_range = chunk.y_range();

            for y in y_range {
                for z in 0..16 {
                    for x in 0..16 {
                        if let Some((block_id, data_value)) = chunk.raw_block(x, y, z) {
                            // Try to get block name from idmap
                            let block_name = palette
                                .get_block_name(block_id, data_value)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| format!("block_{}:{}", block_id, data_value));

                            *blocks_found.entry(block_name).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        Err(_) => {
            // Failed to parse, skip this chunk
        }
    }

    Ok(())
}

pub fn execute(args: AnalyzeArgs) -> Result<()> {
    info!("Starting block analysis");
    info!("Region path: {}", args.region.display());

    if !args.region.exists() {
        error!("Region path does not exist: {}", args.region.display());
        return Err(format!("Region path not found: {}", args.region.display()).into());
    }

    // Load palette
    let palette = get_palette(&args.palette)?;
    info!(
        "Palette contains {} block states",
        palette.blockstates.len()
    );

    // Determine if input is a file or directory
    let coords = if args.region.is_file() {
        // Single region file - parse coordinates from filename
        let filename = args
            .region
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid file name")?;

        // Parse r.X.Z.mca format
        let parts: Vec<&str> = filename.split('.').collect();
        if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
            return Err("Invalid region file name format (expected r.X.Z.mca)".into());
        }

        let x: isize = parts[1].parse()?;
        let z: isize = parts[2].parse()?;

        info!(
            "Analyzing single region file: {} at coordinates ({}, {})",
            filename, x, z
        );
        vec![(RCoord(x), RCoord(z))]
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

        coords
    } else {
        error!(
            "Region path is neither a file nor a directory: {}",
            args.region.display()
        );
        return Err(format!("Invalid region path: {}", args.region.display()).into());
    };

    let region_dir = if args.region.is_file() {
        args.region
            .parent()
            .ok_or("Could not get parent directory")?
            .to_path_buf()
    } else {
        args.region.clone()
    };

    info!("Scanning blocks in {} regions...", coords.len());

    // Collect all blocks found in regions
    let mut blocks_found: HashMap<String, usize> = HashMap::new();
    let mut chunks_scanned = 0;
    let mut regions_scanned = 0;

    for (x, z) in coords {
        let loader = RegionFileLoader::new(region_dir.clone());

        let mut region = match loader.region(x, z)? {
            Some(r) => r,
            None => continue,
        };

        regions_scanned += 1;

        // Scan all chunks in this region
        for chunk_z in 0..32 {
            for chunk_x in 0..32 {
                match region.read_chunk(chunk_x, chunk_z) {
                    Ok(Some(chunk_data)) => {
                        chunks_scanned += 1;
                        if let Err(e) =
                            analyze_chunk_blocks(&chunk_data, &mut blocks_found, &palette)
                        {
                            error!(
                                "Error analyzing chunk ({}, {}) in region ({}, {}): {}",
                                chunk_x, chunk_z, x.0, z.0, e
                            );
                        }
                    }
                    Ok(None) => {
                        // Chunk doesn't exist, skip
                    }
                    Err(e) => {
                        error!(
                            "Error reading chunk ({}, {}) in region ({}, {}): {}",
                            chunk_x, chunk_z, x.0, z.0, e
                        );
                    }
                }
            }
        }

        if regions_scanned % 10 == 0 {
            info!(
                "Scanned {} regions, {} chunks so far...",
                regions_scanned, chunks_scanned
            );
        }
    }

    info!(
        "Scan complete: {} regions, {} chunks",
        regions_scanned, chunks_scanned
    );
    info!("Found {} unique block types", blocks_found.len());

    // Find blocks not in palette
    let mut unknown_blocks: Vec<(String, usize)> = blocks_found
        .iter()
        .filter(|(block_name, _)| !palette.blockstates.contains_key(*block_name))
        .map(|(name, count)| (name.clone(), *count))
        .collect();

    unknown_blocks.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by count descending

    if unknown_blocks.is_empty() {
        info!("✓ All blocks found in regions are present in the palette!");
    } else {
        info!(
            "✗ Found {} block types not in palette:",
            unknown_blocks.len()
        );
        println!("\nBlocks not in palette:");
        println!("{}", "=".repeat(60));

        if args.show_counts {
            for (block_name, count) in &unknown_blocks {
                println!("{:50} {:>8} occurrences", block_name, count);
            }
        } else {
            for (block_name, _) in &unknown_blocks {
                println!("{}", block_name);
            }
        }

        println!("{}", "=".repeat(60));
        println!("Total unknown blocks: {}", unknown_blocks.len());

        if !args.show_counts {
            println!("\nTip: Use --show-counts to see how many times each block appears");
        }
    }

    Ok(())
}
