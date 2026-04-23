# mcmap - Minecraft Map Renderer and Analysis Tool

Fast command-line tool for rendering Minecraft region files and analyzing block usage.

## Installation

### Download Pre-built Binaries

Download the latest release for your platform from [GitHub Releases](https://github.com/xyqyear/mcmap/releases):

### Build from Source

```bash
cargo build --release
```

## Quick Start

```bash
# Render a map (using block colors)
mcmap render --region r.0.0.mca --palette palette.json --output map.png

# Render a heightmap (color-coded by elevation)
mcmap heightmap --region r.0.0.mca --output heightmap.png

# Analyze blocks
mcmap analyze --region /world/region --palette palette.json --show-counts

# Generate palette from Minecraft JAR (and optionally mod jars)
mcmap gen-palette modern -p /path/to/1.20.1.jar --output palette.json
```

## Examples

### Block-colored Render

Renders blocks with their actual colors from the palette:

![Render Example](readme-assets/render_example.png)

### Heightmap Visualization

Color-coded by elevation (default gradient: black → blue → green → red):

![Heightmap Example](readme-assets/heightmap_example.png)

## Commands

### `render` - Render region files to PNG maps

- Supports **1.13+** chunk format (fastanvil), **1.7.10** (with optional NotEnoughIDs extended block IDs), and **Forge 1.12.2** (with RoughlyEnoughIDs / JustEnoughIDs per-section palette format)
- Auto-detects the palette format — modern palette routes through the 1.13+ pipeline, `"format":"1.7.10"` triggers the 1.7.10 legacy path, `"format":"1.12.2"` triggers the REI/JEID legacy path
- Parallel processing for multiple regions
- Output to file or stdout (for HTTP APIs)

```bash
# Basic rendering
mcmap render -r region.mca -p palette.json -o map.png

# Stdout output (e.g. for Python/HTTP integration)
mcmap render -r region.mca -p palette.json -o -

# Split mode: save each region as its own PNG inside a directory
# (names mirror the region's .mca file, e.g. r.0.0.mca -> r.0.0.png)
mcmap render -r /world/region -p palette.json -o ./tiles --split

# Copy each source .mca's mtime onto its PNG (only with --split).
# Useful for incremental re-renders driven by file mtimes.
mcmap render -r /world/region -p palette.json -o ./tiles --split --preserve-mtime
```

### `heightmap` - Render height-based heatmaps

Generates color-coded elevation maps from region files, where colors represent terrain height.

**Features:**

- Two height modes:
  - **Trust heightmap** (default): Uses pre-computed heightmap data from chunks for fast rendering
  - **Calculate heights** (`--calculate-heights`): Scans all blocks to find surface height (slower but more accurate)
- Linear interpolation between color points for smooth gradients
- Custom color mapping support via JSON
- Parallel processing for multiple regions
- Output to file or stdout

**Default color mapping:**

- `-64` (bedrock level): Black
- `0` (sea level): Blue
- `128`: Green
- `255` (old build height): Red

**Basic usage:**

```bash
# Single region file with default colors
mcmap heightmap -r r.0.0.mca -o heightmap.png

# Entire region directory
mcmap heightmap -r /world/region -o heightmap.png

# Calculate heights instead of trusting heightmap data
mcmap heightmap -r r.0.0.mca -o heightmap.png --calculate-heights

# Output to stdout
mcmap heightmap -r r.0.0.mca -o -
```

**Custom color mapping:**

```bash
# Custom gradient: deep blue (-64) -> cyan (0) -> yellow (128) -> red (255)
mcmap heightmap -r r.0.0.mca -o heightmap.png \
  --colors '[[-64,0,0,139,255],[0,0,255,255,255],[128,255,255,0,255],[255,255,0,0,255]]'
```

Color format: `[[height, r, g, b, a], ...]`

- Each point defines a height and its corresponding RGBA color
- Heights between points use linear interpolation
- Must have at least one color point
- Points are automatically sorted by height

### `analyze` - Find unknown blocks

- Scans regions to identify all blocks
- Compares against palette to find missing blocks
- Shows occurrence counts

```bash
# Find unknown blocks
mcmap analyze -r /world/region -p palette.json

# Show counts
mcmap analyze -r /world/region -p palette.json --show-counts
```

### `gen-palette` - Generate block → color palette

One command, three version subcommands — pick whichever matches the target world:

- `gen-palette modern` — 1.13+ worlds. Walks blockstate/model/texture JSONs inside `.jar` / `.zip` packs.
- `gen-palette legacy` — 1.7.10 worlds, optionally with NotEnoughIDs (NEID). Uses the world's `level.dat` FML block registry plus a hand-curated vanilla `(name, meta) → texture` table; modded blocks fall back to filename matching.
- `gen-palette forge112` — Forge 1.12.2 worlds running RoughlyEnoughIDs / JEID. Reads the modern `FML.Registries.minecraft:blocks.ids` registry, then runs the modern blockstate resolver for modded blocks (1.12.2 already ships blockstate/model JSONs) alongside the shared vanilla table.

Common traits across all three:

- Reads from `.jar` / `.zip` packs directly — no extraction step.
- Multiple packs layer, first-listed wins on conflict (list custom resource packs first, vanilla last).
- Recurses into `META-INF/jarjar/*.jar` (Forge's Jar-in-Jar bundling).
- `--overrides <FILE>` applies last, beating every automatic tier; format is a JSON map of the palette's key scheme (`"ns:id"` for modern, `"id"` / `"id|meta"` for legacy/forge112) → `[r,g,b,a]`.
- Transparent pixels are skipped when averaging RGB so sparse textures (vines, fences, crops, rails) keep their real color instead of being pulled toward black.

The output palette's top-level shape tells `render` which chunk codec to use: flat `{"ns:name": [r,g,b,a]}` ⇒ modern; wrapped `{"format":"1.7.10" | "1.12.2", "blocks": {...}}` ⇒ legacy (with the format tag distinguishing the two on-disk chunk shapes).

#### `gen-palette modern` — 1.13+

**Resolution tiers** (first success wins per blockstate):

1. Render the top face of the block's model (`fastanvil` renderer).
2. Raw-model fallback: any face (`up`→`down`→sides) from the variant's model, from any other variant of the same block (preferring `upper`/`top` keys for tall plants and double slabs), or from the first `apply` model of a multipart blockstate.
3. Regex rewrites — generic patterns (`*:*_fence` → `*:block/*_planks`, same for walls and fence gates) apply across any namespace; hardcoded vanilla quirks (crops at final stage, `fire_0`, `bamboo_stalk`) apply to `minecraft:` only.
4. Texture-path probe — direct lookup of `<ns>:block/<name>`.
5. Substring / generic-blockstate bridges for custom state mappers and dynamically-registered block families.
6. User overrides (`--overrides`) — final authoritative precedence.

Typical vanilla JAR locations:

- Linux: `~/.minecraft/versions/1.20.1/1.20.1.jar`
- Windows: `%APPDATA%\.minecraft\versions\1.20.1\1.20.1.jar`
- macOS: `~/Library/Application Support/minecraft/versions/1.20.1/1.20.1.jar`

**Examples:**

```bash
# Vanilla only
mcmap gen-palette modern -p ~/.minecraft/versions/1.20.1/1.20.1.jar -o palette.json

# Vanilla + a mod jar (mod blocks appear as `create:cogwheel`, etc.)
mcmap gen-palette modern \
  -p create-0.5.jar \
  -p ~/.minecraft/versions/1.20.1/1.20.1.jar \
  -o palette.json

# Point at your server's mods directory (every .jar inside is loaded)
mcmap gen-palette modern -p ./server/mods -p 1.20.1.jar -o palette.json

# Custom resource pack overrides vanilla block colors
mcmap gen-palette modern -p my_pack.zip -p 1.20.1.jar -o palette.json
```

#### `gen-palette legacy` — 1.7.10 (optionally NEID)

1.7.10 has no blockstate/model JSONs — block rendering is hard-coded in Java. The legacy path works as follows:

1. Reads the FML block registry from the world's `level.dat` (numeric id → `namespace:name`, world-specific and assigned at first world generation).
2. For each registered block:
   - If `minecraft:*`, looks it up in a hand-curated `(name, meta) → texture_path` table covering the 100+ common 1.7.10 terrain blocks (shared with `forge112` under `src/commands/gen_palette/shared/vanilla_1x.rs`).
   - Otherwise, filename-matches the local name against `assets/<namespace>/textures/blocks/*.png` in the mod jars (exact → case-insensitive → stripped-prefix → fuzzy substring).
3. Averages the resolved texture, applies vanilla biome tints (grass/leaves/vines) + water/lava/air overrides, emits a JSON palette keyed by `"id|meta"` or bare `"id"`.

NotEnoughIDs chunks (with `Blocks16` / `Data16` for 16-bit ids) are handled transparently by the renderer — no flag needed.

**Example (a GTNH world):**

```bash
mcmap gen-palette legacy \
    --level-dat /path/to/gtnh-world/level.dat \
    --pack ~/.minecraft/versions/'GT New Horizons'/mods \
    --pack ~/.minecraft/versions/'GT New Horizons'/1.7.10.jar \
    --output gtnh-palette.json

mcmap render -r /path/to/gtnh-world/region -p gtnh-palette.json -o map.png
```

Mod block → texture matching is best-effort. Many modded blocks with non-obvious internal names (GregTech machines, Thaumcraft runic blocks) fall back to a generic gray. Use `--overrides` with a `{"id|meta": [r,g,b,a]}` JSON to pin specific blocks manually.

#### `gen-palette forge112` — Forge 1.12.2 + REI / JEID

[RoughlyEnoughIDs](https://github.com/MineCrak/RoughlyEnoughIDs) (REI) and its predecessor [JustEnoughIDs](https://github.com/DimensionalDevelopment/JustEnoughIDs) (JEID) write a per-section block-state palette into each chunk and lift the 4096 numeric block-id ceiling to `Integer.MAX_VALUE - 1`. The on-disk shape is partway between vanilla 1.7.10 and 1.13+ — see [`docs/forge_1_12_2_rei.md`](./docs/forge_1_12_2_rei.md) for the full spec.

Pipeline:

1. Reads the modern FML registry (`FML.Registries.minecraft:blocks.ids`) from `level.dat`.
2. Vanilla (`minecraft:*`) blocks reuse the shared `(name, meta) → texture` table. Texture filenames under `assets/minecraft/textures/blocks/` are stable between 1.7.10 and 1.12.2; the lookup probes both `block/` and `blocks/` forms automatically.
3. Modded blocks run the modern blockstate-aware resolver (same code as `gen-palette modern`). Forge's `forge_marker: 1` blockstate format is recognized — the default model is extracted and resolved through the parent chain alongside standard blockstates.
4. Applies vanilla biome tints (grass, leaves, vines) + special blocks (water/lava/air) keyed by registered name.
5. Emits `{"format":"1.12.2", "blocks": {"id": [r,g,b,a], "id|meta": [r,g,b,a], ...}}`. The `render` command auto-routes to the REI chunk decoder.

**Example (a Nova-style 1.12.2 modpack):**

```bash
mcmap gen-palette forge112 \
    --level-dat /path/to/world/level.dat \
    --pack /path/to/modpack/mods \
    --pack /path/to/modpack/1.12.2.jar \
    --output nova-palette.json

mcmap render -r /path/to/world/region -p nova-palette.json -o map.png
```

Same fallback-gray caveat as `gen-palette legacy` — use `--overrides` to pin specific blocks.

## External Stdout Integration

Both `render` and `heightmap` commands support stdout output for integration with web frameworks and other tools.

```python
import subprocess

# Render block-colored map and get PNG data
result = subprocess.run(
    ["mcmap", "render", "-r", "region.mca", "-p", "palette.json", "-o", "-"],
    stdout=subprocess.PIPE
)
png_data = result.stdout

# Use in Flask/FastAPI
from flask import send_file
from io import BytesIO
return send_file(BytesIO(png_data), mimetype='image/png')
```

## Performance

Performance benchmarks on a 512×512 region:

- **Render**: ~470ms (includes block color lookup)
- **Heightmap** (trust mode): ~210ms (uses existing heightmap data in the region file)
- **Heightmap** (calculate mode): ~330ms (scans all blocks)

## License

This project uses `fastanvil` and `fastnbt` libraries for Minecraft data processing.

Some code in this project is adapted from the [fastnbt](https://github.com/owengage/fastnbt) project.
