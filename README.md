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

# Generate palette from Minecraft JAR
mcmap gen-palette --assets /path/to/minecraft/assets/minecraft --output palette.json
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

- Supports 1.7.10+ (Pre-1.13) and 1.13+ (Post-1.13) chunk formats
- Parallel processing for multiple regions
- Output to file or stdout (for HTTP APIs)

```bash
# Basic rendering
mcmap render -r region.mca -p palette.json -o map.png

# Stdout output (e.g. for Python/HTTP integration)
mcmap render -r region.mca -p palette.json -o -
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

### `gen-palette` - Generate palette from Minecraft JAR assets

- Extracts block colors from Minecraft JAR textures
- Automatically adds missing common blocks (water, air, vine, grass, fern, etc.)
- Automatically adds base colors for block state variants (for O(1) lookup performance)
- Outputs `palette.json` only (no grass/foliage colormaps)

**How to extract Minecraft JAR assets:**

```bash
# 1. Locate your Minecraft JAR (example paths)
# Linux: ~/.minecraft/versions/1.20.1/1.20.1.jar
# Windows: %APPDATA%\.minecraft\versions\1.20.1\1.20.1.jar
# macOS: ~/Library/Application Support/minecraft/versions/1.20.1/1.20.1.jar

# 2. Extract the JAR file (it's just a ZIP)
unzip ~/.minecraft/versions/1.20.1/1.20.1.jar -d /tmp/minecraft_jar

# 3. Generate palette from the extracted assets
mcmap gen-palette --assets /tmp/minecraft_jar/assets/minecraft --output palette.json
```

**Example usage:**

```bash
# Generate palette for Minecraft 1.20.1
mcmap gen-palette -a /tmp/minecraft_jar/assets/minecraft -o palette.json

# The generated palette.json will include:
# - All block states with rendered colors
# - Missing common blocks (water, air, etc.) added automatically
# - Base colors for state variants (e.g., minecraft:grass_block)
```

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
