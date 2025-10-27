# mcmap - Minecraft Map Renderer and Analysis Tool

Fast command-line tool for rendering Minecraft region files and analyzing block usage.

## Quick Start

```bash
# Build
cargo build --release

# Render a map (using simplified palette)
./target/release/mcmap render --region r.0.0.mca --palette palette.json --output map.png

# Analyze blocks
./target/release/mcmap analyze --region /world/region --palette palette.json --show-counts
```

## 📁 Project Files

- `palette.json` - Block name to color mapping (4695 blocks)
- `idmap.json` - Pre-1.13 block ID to name mapping (1752 IDs)
- `PALETTE_SYSTEM.md` - Detailed palette system documentation
- `OPTIMIZATION_SUMMARY.md` - Performance optimization record
- `IMPROVEMENTS.md` - Chunk parsing improvements

## Commands

### `render` - Render region files to PNG maps

- Supports 1.7.10+ (Pre-1.13) and 1.13+ (Post-1.13) chunk formats
- Parallel processing for multiple regions
- Output to file or stdout (for HTTP APIs)

```bash
# Basic rendering
mcmap render -r region.mca -p palette.tar.gz -o map.png

# Stdout output (for Python/HTTP integration)
mcmap render -r region.mca -p palette.tar.gz -o -
```

### `analyze` - Find unknown blocks

- Scans regions to identify all blocks
- Compares against palette to find missing blocks
- Shows occurrence counts

```bash
# Find unknown blocks
mcmap analyze -r /world/region -p palette.tar.gz

# Show counts
mcmap analyze -r /world/region -p palette.tar.gz --show-counts
```

## Python Integration

```python
import subprocess

# Render and get PNG data
result = subprocess.run(
    ["mcmap", "render", "-r", "region.mca", "-p", "palette.tar.gz", "-o", "-"],
    stdout=subprocess.PIPE
)
png_data = result.stdout

# Use in Flask/FastAPI
from flask import send_file
from io import BytesIO
return send_file(BytesIO(png_data), mimetype='image/png')
```

## Project Structure

```text
src/
├── main.rs              # CLI entry point with subcommands
├── commands/
│   ├── mod.rs          # Commands module
│   ├── render.rs       # Render subcommand
│   └── analyze.rs      # Analyze subcommand
└── anvil/              # Minecraft format handling
    ├── block.rs        # Block definitions
    ├── chunk.rs        # Chunk parsing (Pre-1.13 & Post-1.13)
    ├── region.rs       # Region file access
    └── render.rs       # Rendering logic
```

## Performance

- **Render**: ~140ms per 512×512 region
- **Analyze**: ~100ms per region with 1000 chunks
- Scales linearly with parallel processing

## Documentation

- `USAGE.md` - Detailed usage guide with examples
- `example_python_usage.py` - Python integration examples

## License

Uses `fastanvil` and `fastnbt` libraries for Minecraft data processing.
