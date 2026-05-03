"""Test the `analyze` subcommand. Modern (1.13+) only — analyze does not
support the legacy chunk codec.
"""

from __future__ import annotations

import json
from pathlib import Path

from asserts import assert_result, overworld_region_dir, run_mcmap_json
from flavors import Flavor
from server import ServerInstance
from palette import palette_for


def _setup_world_with_stone(work_dir: Path, flavor: Flavor) -> Path:
    with ServerInstance(flavor, work_dir) as srv:
        srv.setblock(0, 64, 0, "minecraft:stone")
        srv.setblock(0, 64, 1, "minecraft:dirt")
    return overworld_region_dir(work_dir)


def test_analyze_full_palette_has_no_unknowns(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    region_dir = _setup_world_with_stone(work_dir, _flavor)
    palette_path = palette_for(_flavor, run_root)
    events, rc = run_mcmap_json([
        "analyze",
        "-r", str(region_dir),
        "-p", str(palette_path),
    ])
    assert rc == 0, f"analyze failed: {events!r}"
    result = assert_result(events)
    # In a flat-bedrock world with /setblock'd vanilla blocks against a
    # vanilla palette, there should be no unknown blocks at all.
    assert result["unknown_blocks"] == [], (
        f"unexpected unknowns: {result['unknown_blocks']!r}"
    )


def test_analyze_stripped_palette_flags_missing_block(
    work_dir: Path, _flavor: Flavor, run_root: Path
) -> None:
    region_dir = _setup_world_with_stone(work_dir, _flavor)
    full_palette = palette_for(_flavor, run_root)
    palette = json.loads(full_palette.read_text())
    # Drop dirt; analyze should now report it as unknown.
    palette.pop("minecraft:dirt", None)
    stripped = work_dir / "stripped-palette.json"
    stripped.write_text(json.dumps(palette))

    events, rc = run_mcmap_json([
        "analyze",
        "-r", str(region_dir),
        "-p", str(stripped),
    ])
    assert rc == 0, f"analyze failed: {events!r}"
    result = assert_result(events)
    names = {entry["name"] for entry in result["unknown_blocks"]}
    assert "minecraft:dirt" in names, (
        f"expected dirt to be flagged unknown; got {names!r}"
    )


# Narrow these tests to modern-palette flavors.
test_analyze_full_palette_has_no_unknowns.applicable_flavors = ("vanilla-latest",)  # type: ignore[attr-defined]
test_analyze_stripped_palette_flags_missing_block.applicable_flavors = ("vanilla-latest",)  # type: ignore[attr-defined]
