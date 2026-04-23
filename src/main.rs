mod anvil;
mod commands;

use clap::{Parser, Subcommand};
use env_logger::Env;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser, Debug)]
#[command(author, version, about = "Minecraft map renderer and analysis tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Render region files to PNG overhead maps
    Render(commands::render::RenderArgs),

    /// Render region files as height-based heatmaps
    Heightmap(commands::heightmap::HeightmapArgs),

    /// Analyze blocks in region files and find unknown blocks
    Analyze(commands::analyze::AnalyzeArgs),

    /// Generate palette.json from Minecraft JAR assets (1.13+)
    GenPalette(commands::gen_palette::GenPaletteArgs),

    /// Generate palette.json for a pre-1.13 world (1.7.10, optionally NEID).
    /// Requires the world's level.dat and the mod jars loaded in that world.
    GenPaletteLegacy(commands::gen_palette_legacy::GenPaletteLegacyArgs),

    /// Generate palette.json for a Forge 1.12.2 world (RoughlyEnoughIDs / JEID
    /// per-section palette format). Requires the world's level.dat and the mod
    /// jars loaded in that world.
    GenPaletteForge112(commands::gen_palette_forge112::GenPaletteForge112Args),
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Render(args) => commands::render::execute(args),
        Commands::Heightmap(args) => commands::heightmap::execute(args),
        Commands::Analyze(args) => commands::analyze::execute(args),
        Commands::GenPalette(args) => commands::gen_palette::execute(args),
        Commands::GenPaletteLegacy(args) => commands::gen_palette_legacy::execute(args),
        Commands::GenPaletteForge112(args) => commands::gen_palette_forge112::execute(args),
    }
}
