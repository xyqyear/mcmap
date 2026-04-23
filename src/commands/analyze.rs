// Analyze command — finds blocks not present in the palette. 1.13+ only.

use clap::Args;
use log::{error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::util::parse_region_filename;
use crate::anvil::modern::ChunkData;
use crate::anvil::region::RegionLoader;
use crate::anvil::{RegionFileLoader, RenderedPalette, Rgba};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Path to a region folder or a single .mca file
    #[arg(short, long)]
    region: PathBuf,

    /// Path to the palette.json file
    #[arg(short, long)]
    palette: PathBuf,

    /// Show block counts (how many times each unknown block appears)
    #[arg(long, default_value_t = false)]
    show_counts: bool,
}

fn get_palette(path: &Path) -> Result<RenderedPalette> {
    info!("Loading palette from: {}", path.display());
    let bytes = std::fs::read(path)?;
    let blockstates: HashMap<String, Rgba> = serde_json::from_slice(&bytes)?;
    info!(
        "Palette loaded successfully: {} block states",
        blockstates.len()
    );
    Ok(RenderedPalette::new(blockstates))
}

fn analyze_chunk_blocks(
    chunk_data: &[u8],
    blocks_found: &mut HashMap<String, usize>,
) -> Result<()> {
    let Ok(chunk) = ChunkData::from_bytes(chunk_data) else {
        return Ok(());
    };

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
    Ok(())
}

pub fn execute(args: AnalyzeArgs) -> Result<()> {
    info!("Starting block analysis");
    info!("Region path: {}", args.region.display());

    if !args.region.exists() {
        error!("Region path does not exist: {}", args.region.display());
        return Err(format!("Region path not found: {}", args.region.display()).into());
    }

    let palette = get_palette(&args.palette)?;
    info!(
        "Palette contains {} block states",
        palette.blockstates.len()
    );

    let coords = if args.region.is_file() {
        let filename = args
            .region
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid file name")?;
        let (x, z) = parse_region_filename(filename)
            .ok_or("Invalid region file name format (expected r.X.Z.mca)")?;
        info!(
            "Analyzing single region file: {} at coordinates ({}, {})",
            filename, x.0, z.0
        );
        vec![(x, z)]
    } else if args.region.is_dir() {
        info!("Loading regions from directory: {}", args.region.display());
        let loader = RegionFileLoader::new(args.region.clone());
        let coords = loader.list()?;
        info!("Found {} region files", coords.len());
        if coords.is_empty() {
            return Err(format!("No region files found in {}", args.region.display()).into());
        }
        coords
    } else {
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

        for chunk_z in 0..32 {
            for chunk_x in 0..32 {
                match region.read_chunk(chunk_x, chunk_z) {
                    Ok(Some(chunk_data)) => {
                        chunks_scanned += 1;
                        if let Err(e) = analyze_chunk_blocks(&chunk_data, &mut blocks_found) {
                            error!(
                                "Error analyzing chunk ({}, {}) in region ({}, {}): {}",
                                chunk_x, chunk_z, x.0, z.0, e
                            );
                        }
                    }
                    Ok(None) => {}
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

    let mut unknown_blocks: Vec<(String, usize)> = blocks_found
        .iter()
        .filter(|(block_name, _)| !palette.blockstates.contains_key(*block_name))
        .map(|(name, count)| (name.clone(), *count))
        .collect();
    unknown_blocks.sort_by(|a, b| b.1.cmp(&a.1));

    if unknown_blocks.is_empty() {
        info!("All blocks found in regions are present in the palette.");
    } else {
        info!(
            "Found {} block types not in palette:",
            unknown_blocks.len()
        );
        let separator: String = "=".repeat(60);
        println!("\nBlocks not in palette:");
        println!("{}", separator);

        if args.show_counts {
            for (block_name, count) in &unknown_blocks {
                println!("{:50} {:>8} occurrences", block_name, count);
            }
        } else {
            for (block_name, _) in &unknown_blocks {
                println!("{}", block_name);
            }
        }

        println!("{}", separator);
        println!("Total unknown blocks: {}", unknown_blocks.len());

        if !args.show_counts {
            println!("\nTip: Use --show-counts to see how many times each block appears");
        }
    }

    Ok(())
}
