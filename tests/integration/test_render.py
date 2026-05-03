"""Test the `render` subcommand end-to-end against real generated worlds.

For each flavor: boot the server, place a known block at a known coordinate,
save+stop, render the region with the flavor's palette, then assert the PNG
pixel at the placed-block's world-XZ matches what the palette says.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest

from asserts import (
    assert_error,
    assert_result,
    colors_close,
    find_event,
    overworld_region_dir,
    png_pixel,
    run_mcmap_json,
)
from flavors import Flavor
from level_dat import palette_key_for_block
from palette import palette_for, palette_lookup
from server import ServerInstance


# Three markers placed inside region (0, 0). x and z stay below 16 so they
# all land in chunk (0, 0), which is inside the spawn-loaded area on every
# flavor (legacy spawn relocates to (0, 4, 0); modern force-loads).
MARKERS: dict[tuple[int, int], str] = {
    (0, 0): "minecraft:stone",
    (4, 0): "minecraft:gold_block",
    (0, 4): "minecraft:diamond_block",
}


def _setup_marked_world(work_dir: Path, flavor: Flavor) -> Path:
    """Boot the flavor, place every MARKERS block at y=64, save+stop. Return
    the overworld region dir.
    """
    with ServerInstance(flavor, work_dir) as srv:
        for (x, z), block in MARKERS.items():
            srv.setblock(x, 64, z, block)
    region_dir = overworld_region_dir(work_dir)
    if not region_dir.is_dir():
        raise RuntimeError(f"no region dir at {region_dir} after boot")
    return region_dir


def _level_dat(work_dir: Path) -> Path:
    """Path to the world's level.dat — needed for legacy palette key lookup."""
    return work_dir / "world" / "level.dat"


def test_render_combined_emits_result(work_dir: Path, _flavor: Flavor, run_root: Path) -> None:
    region_dir = _setup_marked_world(work_dir, _flavor)
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
    assert result["mode"] == "combined"
    assert result["regions_saved"] >= 1
    assert "elapsed_ms" in result
    assert out.exists() and out.stat().st_size > 0


def test_render_split_per_region_pngs(work_dir: Path, _flavor: Flavor, run_root: Path) -> None:
    region_dir = _setup_marked_world(work_dir, _flavor)
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
    region_event = find_event(events, type="region", x=0, z=0)
    assert region_event is not None and region_event["status"] == "rendered", (
        f"region (0,0) not rendered: {events!r}"
    )
    assert (tiles / "r.0.0.png").exists()


def test_render_pixel_matches_palette(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """Per-flavor: render with the flavor's palette and assert each marker
    pixel matches the palette entry. For modern the key is `minecraft:<name>`;
    for legacy/forge112 we resolve `<id>|0` via the world's FML registry.
    """
    region_dir = _setup_marked_world(work_dir, _flavor)
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

    png_path = out / "r.0.0.png"
    for (x, z), name in MARKERS.items():
        # Stone is the default-fill at this y on flat worlds. On legacy
        # flavors the rendered stone pixel may equal the palette stone color
        # by definition — that's a tautology, not a contrast test. Skip.
        if name == "minecraft:stone":
            continue
        key = palette_key_for_block(_flavor.palette_format, _level_dat(work_dir), name)
        rgba = palette_lookup(palette_path, key)
        assert rgba is not None, f"palette missing key {key!r} for {name}"
        # Render is 1 px per block, top-down. World coords (x, z) within
        # region (0,0) map directly to PNG pixel (x, z).
        pixel = png_pixel(png_path, x, z)
        assert colors_close(pixel, rgba, tol=8), (
            f"{_flavor.id}: pixel at ({x},{z}) for {name} (key={key!r}) "
            f"is {pixel}, expected ~{rgba}"
        )


def test_render_calculate_heights_matches_trust_on_flat_world(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """--calculate-heights and the default heightmap-trust path must agree on
    a flat world.

    The two paths *can* legitimately diverge when the heightmap NBT lies
    (e.g. corrupted save), but on a fresh /setblock world both should pick
    the topmost solid block. This is a regression test for the heightmap
    calculation code path that's otherwise never exercised by the suite.
    Modern-only: legacy renderer ignores --calculate-heights.
    """
    if _flavor.palette_format != "modern":
        pytest.skip("--calculate-heights affects modern engine only")
    region_dir = _setup_marked_world(work_dir, _flavor)
    palette = palette_for(_flavor, run_root)

    out_trust = work_dir / "trust.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir), "-p", str(palette), "-o", str(out_trust),
    ])
    assert rc == 0, f"trust render failed: {events!r}"
    assert_result(events)

    out_calc = work_dir / "calc.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir), "-p", str(palette),
        "-o", str(out_calc), "--calculate-heights",
    ])
    assert rc == 0, f"calculate-heights render failed: {events!r}"
    assert_result(events)

    # Sample the marker pixels in both PNGs — they should match exactly.
    for (x, z), _ in MARKERS.items():
        a = png_pixel(out_trust, x, z)
        b = png_pixel(out_calc, x, z)
        assert a == b, (
            f"trust vs calculate-heights diverged at ({x},{z}): {a} vs {b}"
        )


def test_render_preserve_mtime_split(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """--preserve-mtime copies each .mca's mtime onto its .png.

    Touch the source region to a known mtime, render with --split
    --preserve-mtime, assert the PNG's mtime matches (within filesystem
    timestamp granularity).
    """
    region_dir = _setup_marked_world(work_dir, _flavor)
    palette = palette_for(_flavor, run_root)
    src = region_dir / "r.0.0.mca"
    assert src.exists()
    # Pick an arbitrary epoch in the past with a clean second boundary.
    target_mtime = 1_700_000_000.0
    os.utime(src, (target_mtime, target_mtime))

    tiles = work_dir / "tiles"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir), "-p", str(palette),
        "-o", str(tiles), "--split", "--preserve-mtime",
    ])
    assert rc == 0, f"render failed: {events!r}"
    assert_result(events)
    png = tiles / "r.0.0.png"
    assert png.exists()
    # Allow up to 2s of slack — some filesystems have 1s mtime resolution.
    got = png.stat().st_mtime
    assert abs(got - target_mtime) <= 2, (
        f"PNG mtime {got} not close to source mtime {target_mtime}"
    )


def test_render_threads_one_smoke(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """`--threads 1` must produce the same output as the default. Asserts the
    custom thread-pool config path doesn't deadlock or crash under -j 1.
    """
    region_dir = _setup_marked_world(work_dir, _flavor)
    palette = palette_for(_flavor, run_root)
    tiles = work_dir / "tiles"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir), "-p", str(palette),
        "-o", str(tiles), "--split", "-j", "1",
    ])
    assert rc == 0, f"render -j 1 failed: {events!r}"
    result = assert_result(events)
    assert result["regions_saved"] >= 1
    assert (tiles / "r.0.0.png").exists()


def test_render_combined_multi_region(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """Pass two -r dirs containing regions at different coords; assert the
    combined PNG covers both via the bounds event.
    """
    region_dir = _setup_marked_world(work_dir, _flavor)
    src = region_dir / "r.0.0.mca"
    assert src.exists()

    # Stage a copy of r.0.0.mca renamed to r.1.0.mca in a second dir. The
    # byte-identical chunks now sit at region (1, 0). The renderer doesn't
    # care that the chunk-internal coords don't match the file name — it
    # places the 512x512 tile based on the file name, so the combined image
    # spans 1024x512 px.
    extra_dir = work_dir / "extra"
    extra_dir.mkdir()
    shutil.copy2(src, extra_dir / "r.1.0.mca")

    out = work_dir / "combined.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir), "-r", str(extra_dir),
        "-p", str(palette_for(_flavor, run_root)),
        "-o", str(out),
    ])
    assert rc == 0, f"render multi-region failed: {events!r}"
    assert_result(events)

    bounds_event = find_event(events, type="progress", phase="regions_listed")
    assert bounds_event is not None, f"no regions_listed event: {events!r}"
    bounds = bounds_event.get("bounds")
    assert bounds is not None, f"regions_listed has no bounds: {bounds_event!r}"
    # xmax/zmax are exclusive (bounds is `xmin..xmax`).
    assert bounds["xmin"] <= 0 and bounds["xmax"] >= 2, f"unexpected bounds: {bounds}"

    # And the resulting PNG must be at least 2 regions wide.
    from PIL import Image
    with Image.open(out) as im:
        assert im.size[0] >= 1024, f"combined PNG width too small: {im.size}"


# --- Error paths -----------------------------------------------------------


def test_render_missing_region_dir_emits_error(work_dir: Path, _flavor: Flavor) -> None:
    """Non-existent -r path → type=error event, non-zero exit."""
    out = work_dir / "map.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(work_dir / "no-such-dir"),
        "-p", str(work_dir / "no-such-palette.json"),
        "-o", str(out),
    ])
    assert rc != 0
    err = assert_error(events)
    # Error message should reference the missing path or "not found" — be
    # lenient about wording but require *something* informative.
    assert err["message"], f"empty error message: {err!r}"


def test_render_missing_palette_emits_error(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    """Existing region dir + non-existent palette → type=error."""
    region_dir = _setup_marked_world(work_dir, _flavor)
    out = work_dir / "map.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir),
        "-p", str(work_dir / "no-such-palette.json"),
        "-o", str(out),
    ])
    assert rc != 0
    assert_error(events)


def test_render_preserve_mtime_without_split_rejected(work_dir: Path, _flavor: Flavor) -> None:
    """--preserve-mtime without --split is a clap-level rejection.

    clap exits 2 before mcmap's JSON output engages, so we only assert the
    exit code and that no `result` event was emitted.
    """
    region_dir = work_dir / "fake-region"
    region_dir.mkdir()
    out = work_dir / "map.png"
    events, rc = run_mcmap_json([
        "render", "-r", str(region_dir),
        "-p", str(work_dir / "fake-palette.json"),
        "-o", str(out), "--preserve-mtime",
    ])
    assert rc != 0, f"expected rejection, got rc={rc}, events={events!r}"
    assert not any(e.get("type") == "result" for e in events), (
        f"unexpected result event: {events!r}"
    )
