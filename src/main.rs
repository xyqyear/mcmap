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

    /// Generate palette.json from Minecraft JAR assets
    GenPalette(commands::gen_palette::GenPaletteArgs),
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
    }
}
