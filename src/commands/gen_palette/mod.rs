// `gen-palette` — one command, three internal pipelines.
//
// The right pipeline is auto-detected from the world's `level.dat`:
//
//   - `FML.Registries.minecraft:blocks` present → Forge 1.12.2 + REI/JEID
//   - else `FML.ItemData` present              → Forge 1.7.10 (optionally NEID)
//   - else (or no level.dat passed)            → 1.13+ (modern)
//
// `--level-dat` is **required** for pre-1.13 worlds and **ignored** for 1.13+
// — pass it unconditionally and the tool picks the right pipeline. The
// version-specific pipelines live in:
//
//   - `modern.rs`     — 1.13+: walks blockstate/model/texture JSONs.
//   - `legacy_v17.rs` — 1.7.10: FML.ItemData + 1.x texture table + filename
//                       fuzzy match. Output keyed by numeric id.
//   - `forge_v12.rs`  — 1.12.2 + REI: FML.Registries + 1.x texture table for
//                       vanilla + modern blockstate resolver for modded.
//                       Output keyed by numeric id.

pub mod forge_v12;
pub mod legacy_pack;
pub mod legacy_v17;
pub mod leveldat;
pub mod modern;
pub mod modern_pack;
pub mod shared;

use clap::Args;
use log::info;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args, Debug)]
pub struct GenPaletteArgs {
    /// Resource pack: a .jar/.zip file or a directory containing them.
    /// Repeatable — first-listed wins on conflict (custom packs first,
    /// vanilla last). Mods are loaded the same way as resource packs.
    #[arg(short, long, required = true)]
    pub pack: Vec<PathBuf>,

    /// Output palette.json file path.
    #[arg(short, long, default_value = "palette.json")]
    pub output: PathBuf,

    /// Path to the world's `level.dat`. **Required** for pre-1.13 worlds
    /// (Forge 1.7.10 / Forge 1.12.2). **Ignored** for 1.13+. Pass it
    /// unconditionally — the tool picks the right pipeline based on the
    /// FML structure inside.
    #[arg(short, long)]
    pub level_dat: Option<PathBuf>,

    /// Optional user overrides — JSON map of palette key → `[r,g,b,a]`.
    /// Applied last; overrides everything automatic. Key shape depends on
    /// the detected variant: `"namespace:name"` for modern, `"id"` /
    /// `"id|meta"` for legacy / forge112.
    #[arg(long)]
    pub overrides: Option<PathBuf>,
}

/// Which palette pipeline to run. Selected by `leveldat::detect_variant`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PaletteVariant {
    Modern,
    Legacy,
    Forge112,
}

pub fn execute(args: GenPaletteArgs) -> Result<()> {
    let variant = leveldat::detect_variant(args.level_dat.as_deref())?;
    info!("Detected palette variant: {:?}", variant);
    match variant {
        PaletteVariant::Modern => {
            if args.level_dat.is_some() {
                info!("level.dat ignored — 1.13+ pipeline doesn't need it");
            }
            modern::run_modern(&args.pack, &args.output, args.overrides.as_deref())
        }
        PaletteVariant::Legacy => {
            // Detection only returns Legacy when level.dat was passed AND
            // FML.ItemData was present — so unwrap is safe. Defensively turn
            // it into an error message anyway.
            let level_dat = args
                .level_dat
                .as_deref()
                .ok_or("--level-dat is required for 1.7.10 worlds")?;
            legacy_v17::run_legacy_v17(
                &args.pack,
                level_dat,
                &args.output,
                args.overrides.as_deref(),
            )
        }
        PaletteVariant::Forge112 => {
            let level_dat = args
                .level_dat
                .as_deref()
                .ok_or("--level-dat is required for Forge 1.12.2 worlds")?;
            forge_v12::run_forge_v12(
                &args.pack,
                level_dat,
                &args.output,
                args.overrides.as_deref(),
            )
        }
    }
}
