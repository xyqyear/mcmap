mod anvil;
mod commands;
mod output;

use clap::{Parser, Subcommand};
use env_logger::Env;
use serde::Serialize;

#[derive(Parser, Debug)]
#[command(author, version, about = "Minecraft map renderer and analysis tool", long_about = None)]
struct Cli {
    /// Emit machine-readable NDJSON events on stdout instead of human logs.
    /// Each line is one `{"type":"..."}` object; long-running commands emit a
    /// stream of progress events and a final `result` (or `error`).
    #[arg(long, global = true, default_value_t = false)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Render region files to PNG overhead maps. Auto-detects palette format
    /// (modern 1.13+, legacy 1.7.10, or Forge 1.12.2 REI).
    Render(commands::render::RenderArgs),

    /// Analyze blocks in region files and find unknown blocks. 1.13+ only.
    Analyze(commands::analyze::AnalyzeArgs),

    /// Generate palette.json — pick a version subcommand for the world type.
    GenPalette(commands::gen_palette::GenPaletteArgs),

    /// Download the Minecraft client jar for a given version from Mojang.
    DownloadClient(commands::download_client::DownloadClientArgs),
}

#[derive(Serialize)]
struct ErrorEvent<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    message: String,
}

fn main() {
    let cli = Cli::parse();

    output::set_json_mode(cli.json);

    // When --json is on, default env_logger to off so stderr stays clean for
    // callers parsing stdout. Users can re-enable via RUST_LOG.
    let default_filter = if cli.json { "off" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(default_filter))
        .format_timestamp(None)
        .init();

    let result = match cli.command {
        Commands::Render(args) => commands::render::execute(args),
        Commands::Analyze(args) => commands::analyze::execute(args),
        Commands::GenPalette(args) => commands::gen_palette::execute(args),
        Commands::DownloadClient(args) => commands::download_client::execute(args),
    };

    if let Err(e) = result {
        if cli.json {
            output::emit(&ErrorEvent { ty: "error", message: e.to_string() });
        } else {
            eprintln!("Error: {}", e);
        }
        std::process::exit(1);
    }
}
