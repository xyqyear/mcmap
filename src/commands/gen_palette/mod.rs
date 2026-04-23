// `gen-palette` — one command, three versions. Subcommand selects how to
// derive `(id|meta → color)` (legacy) or `(namespace:name → color)` (modern)
// entries:
//
//   - `modern`    — 1.13+ worlds. Walks blockstate/model/texture JSONs.
//   - `legacy`    — 1.7.10 worlds. Hand-curated vanilla table + filename
//                   matching for modded blocks; uses FML.ItemData registry.
//   - `forge112`  — 1.12.2 worlds running RoughlyEnoughIDs / JEID. Uses the
//                   modern blockstate resolver for modded blocks + the same
//                   curated table for vanilla; reads FML.Registries block ids.
//
// See the individual subcommand modules for the pipeline details.

pub mod forge112;
pub mod legacy;
pub mod legacy_pack;
pub mod modern;
pub mod modern_pack;
pub mod shared;

use clap::{Args, Subcommand};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteArgs {
    #[command(subcommand)]
    version: Version,
}

#[derive(Subcommand, Debug)]
enum Version {
    /// Generate palette.json from Minecraft JAR assets (1.13+).
    Modern(modern::ModernArgs),

    /// Generate palette.json for a pre-1.13 world (1.7.10, optionally NEID).
    /// Requires the world's level.dat and the mod jars loaded in that world.
    Legacy(legacy::LegacyArgs),

    /// Generate palette.json for a Forge 1.12.2 world (RoughlyEnoughIDs /
    /// JEID per-section palette format). Requires the world's level.dat and
    /// the mod jars loaded in that world.
    Forge112(forge112::Forge112Args),
}

pub fn execute(args: GenPaletteArgs) -> Result<()> {
    match args.version {
        Version::Modern(a) => modern::execute(a),
        Version::Legacy(a) => legacy::execute(a),
        Version::Forge112(a) => forge112::execute(a),
    }
}
