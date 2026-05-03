"""Test the `render` subcommand end-to-end against real generated worlds.

For each flavor: boot the server, place a known block at a known coordinate,
save+stop, render the region with the flavor's palette, then assert the PNG
pixel at the placed-block's world-XZ matches what the palette says.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from asserts import (
    assert_result,
    colors_close,
    find_event,
    overworld_region_dir,
    png_pixel,
    run_mcmap_json,
)
from flavors import Flavor
from palette import palette_for
from server import ServerInstance


def _setup_marked_world(work_dir: Path, flavor: Flavor) -> tuple[Path, dict[tuple[int, int], str]]:
    """Boot the flavor, place a few marker blocks, save+stop. Return (region_dir, marks).

    `marks` is a dict of (x, z) world-coords → block name.
    """
    marks: dict[tuple[int, int], str] = {
        (0, 0): "minecraft:stone",
        (4, 0): "minecraft:gold_block",
        (0, 4): "minecraft:diamond_block",
    }
    with ServerInstance(flavor, work_dir) as srv:
        for (x, z), block in marks.items():
            srv.setblock(x, 64, z, block)
    region_dir = overworld_region_dir(work_dir)
    if not region_dir.is_dir():
        raise RuntimeError(f"no region dir at {region_dir} after boot")
    return region_dir, marks


def test_render_combined_emits_result(work_dir: Path, _flavor: Flavor, run_root: Path) -> None:
    region_dir, _marks = _setup_marked_world(work_dir, _flavor)
    palette = palette_for(_flavor, run_root)
    out = work_dir / "map.png"
    events, rc = run_mcmap_json([
        "render",
        "-r", str(region_dir),
        "-p", str(palette),
        "-o", str(out),
    ])
    assert rc == 0, f"render failed: {events!r}"
    result = assert_result(events)
    assert result["regions_saved"] >= 1
    assert out.exists() and out.stat().st_size > 0


def test_render_split_per_region_pngs(work_dir: Path, _flavor: Flavor, run_root: Path) -> None:
    region_dir, _marks = _setup_marked_world(work_dir, _flavor)
    palette = palette_for(_flavor, run_root)
    tiles = work_dir / "tiles"
    events, rc = run_mcmap_json([
        "render",
        "-r", str(region_dir),
        "-p", str(palette),
        "-o", str(tiles),
        "--split",
    ])
    assert rc == 0, f"render --split failed: {events!r}"
    result = assert_result(events)
    assert result["mode"] == "split"
    assert result["regions_saved"] >= 1
    # We placed blocks within region (0,0).
    region_event = find_event(events, type="region", x=0, z=0)
    assert region_event is not None and region_event["status"] == "rendered", (
        f"region (0,0) not rendered: {events!r}"
    )
    assert (tiles / "r.0.0.png").exists()


def test_render_modern_pixel_matches_palette(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """Modern-only assertion: rendered pixel at a placed block's coord matches
    the palette's RGB. Legacy palettes are keyed by numeric id and require a
    level.dat-aware lookup; we cover the legacy render path via the smoke
    tests above and the round-trip test in test_replace_chunks instead.
    """
    if _flavor.palette_format != "modern":
        pytest.skip("pixel-vs-palette check is modern-only; legacy keys differ")

    region_dir, marks = _setup_marked_world(work_dir, _flavor)
    palette_path = palette_for(_flavor, run_root)
    out = work_dir / "tiles"
    events, rc = run_mcmap_json([
        "render",
        "-r", str(region_dir),
        "-p", str(palette_path),
        "-o", str(out),
        "--split",
    ])
    assert rc == 0
    assert_result(events)

    palette = json.loads(palette_path.read_text())

    png_path = out / "r.0.0.png"
    for (x, z), name in marks.items():
        # Skip stone — it's the default-fill at this y on flat worlds and
        # doesn't form a meaningful contrast test.
        if name == "minecraft:stone":
            continue
        rgba = palette.get(name)
        assert rgba is not None, f"palette missing {name}"
        # Render is 1 px per block, top-down. World coords (x, z) within
        # region (0,0) map directly to PNG pixel (x, z).
        pixel = png_pixel(png_path, x, z)
        assert colors_close(pixel, tuple(rgba), tol=8), (
            f"pixel at ({x},{z}) for {name} is {pixel}, expected ~{rgba}"
        )
