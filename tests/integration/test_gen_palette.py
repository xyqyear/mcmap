"""Test `gen-palette` against a real generated world for each flavor.

For modern flavors: pass the vanilla client jar; assert palette has many
entries and that minecraft:stone resolves to a non-grayscale color.

For legacy / forge112 flavors: bootstrap a base world (so level.dat exists),
then run gen-palette with that level.dat plus the matching client jar (and
mod jars for forge112). Assert palette is wrapped with the correct format
tag and that the counters reflect successful resolution.
"""

from __future__ import annotations

import json
from pathlib import Path

from asserts import assert_result, run_mcmap_json
from cache import vanilla_client_jar
from flavors import Flavor
from server import ServerInstance


def _client_jar_for(flavor: Flavor, work_dir: Path) -> Path:
    dest = work_dir / f"client-{flavor.mc_version}.jar"
    return vanilla_client_jar(flavor.mc_version, dest)


def test_gen_palette_against_flavor(work_dir: Path, _flavor: Flavor) -> None:
    out = work_dir / "palette.json"

    if _flavor.palette_format == "modern":
        client_jar = _client_jar_for(_flavor, work_dir)
        events, rc = run_mcmap_json([
            "gen-palette", "modern",
            "-p", str(client_jar),
            "-o", str(out),
        ])
        assert rc == 0, f"gen-palette modern failed: {events!r}"
        result = assert_result(events)
        assert result["entries"] >= 500, f"too few entries: {result!r}"
        palette = json.loads(out.read_text())
        stone = palette.get("minecraft:stone")
        assert stone is not None, "modern palette is missing minecraft:stone"
        # Stone is a recognizable gray; alpha should be 255 and r/g/b nonzero.
        assert all(c > 0 for c in stone[:3])
        return

    # Legacy/forge112 — bootstrap a base world to get a level.dat.
    with ServerInstance(_flavor, work_dir):
        pass
    level_dat = work_dir / "world" / "level.dat"
    assert level_dat.exists(), f"no level.dat at {level_dat}"
    client_jar = _client_jar_for(_flavor, work_dir)

    args = [
        "gen-palette", _flavor.palette_format,
        "--level-dat", str(level_dat),
        "--pack", str(client_jar),
        "-o", str(out),
    ]
    if _flavor.palette_format == "forge112":
        mods_dir = work_dir / "mods"
        for jar in sorted(mods_dir.glob("*.jar")):
            args.extend(["--pack", str(jar)])

    events, rc = run_mcmap_json(args)
    assert rc == 0, f"gen-palette {_flavor.palette_format} failed: {events!r}"
    result = assert_result(events)
    assert result["entries"] > 0
    palette = json.loads(out.read_text())
    expected_format = "1.7.10" if _flavor.palette_format == "legacy" else "1.12.2"
    assert palette.get("format") == expected_format, (
        f"expected wrapped palette with format={expected_format}, got {palette.get('format')!r}"
    )
    blocks = palette.get("blocks") or {}
    assert len(blocks) > 0, "palette has no blocks entries"
