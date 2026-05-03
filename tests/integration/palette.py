"""Generate per-flavor palette.json files for tests that need to render or
analyze. Modern palettes need only a client jar; legacy/forge112 need a
freshly generated level.dat from a real world boot of that flavor.

Palettes are cached at the session level; the same flavor's palette is
shared across all tests in a session.
"""

from __future__ import annotations

import logging
import shutil
from pathlib import Path

from asserts import assert_result, run_mcmap_json
from cache import vanilla_client_jar
from flavors import Flavor
from server import ServerInstance

log = logging.getLogger(__name__)


def _palette_cache_dir(run_root: Path) -> Path:
    p = run_root / "palettes"
    p.mkdir(parents=True, exist_ok=True)
    return p


def _client_jar_for(flavor: Flavor, run_root: Path) -> Path:
    """Vanilla client jar for the flavor's mc_version, cached per session."""
    cdir = run_root / "client-jars"
    cdir.mkdir(parents=True, exist_ok=True)
    dest = cdir / f"client-{flavor.mc_version}.jar"
    return vanilla_client_jar(flavor.mc_version, dest)


def _bootstrap_world(flavor: Flavor, dest: Path) -> Path:
    """Boot the flavor's server in `dest`, save+stop. Returns dest/world."""
    log.info("Bootstrapping base world for %s in %s", flavor.id, dest)
    with ServerInstance(flavor, dest):
        # __exit__ runs save_and_stop. No commands needed; default world gen
        # creates level.dat + the FML registry.
        pass
    world = dest / "world"
    if not (world / "level.dat").exists():
        raise RuntimeError(f"base world bootstrap did not produce level.dat in {world}")
    return world


def palette_for(flavor: Flavor, run_root: Path) -> Path:
    """Return a palette.json suitable for rendering this flavor's worlds.

    Cached per-session under run_root/palettes/<flavor.id>.json.
    """
    out = _palette_cache_dir(run_root) / f"{flavor.id}.json"
    if out.exists():
        return out

    if flavor.palette_format == "modern":
        client_jar = _client_jar_for(flavor, run_root)
        events, rc = run_mcmap_json([
            "gen-palette", "modern",
            "-p", str(client_jar),
            "-o", str(out),
        ])
        if rc != 0:
            raise RuntimeError(f"gen-palette modern failed for {flavor.id}: {events!r}")
        assert_result(events)
        return out

    # legacy / forge112: need a level.dat from this flavor's world plus the
    # client jar of the same MC version (and the mod jars for forge112).
    base_dir = run_root / "palette-base" / flavor.id
    base_dir.mkdir(parents=True, exist_ok=True)
    world = _bootstrap_world(flavor, base_dir)
    client_jar = _client_jar_for(flavor, run_root)

    args: list[str]
    if flavor.palette_format == "legacy":
        args = [
            "gen-palette", "legacy",
            "--level-dat", str(world / "level.dat"),
            "--pack", str(client_jar),
            "-o", str(out),
        ]
    elif flavor.palette_format == "forge112":
        args = [
            "gen-palette", "forge112",
            "--level-dat", str(world / "level.dat"),
            "--pack", str(client_jar),
            "-o", str(out),
        ]
        # Add mod jars from the bootstrapped server's mods/ dir.
        mods_dir = base_dir / "mods"
        if mods_dir.is_dir():
            for jar in sorted(mods_dir.glob("*.jar")):
                args.extend(["--pack", str(jar)])
    else:
        raise RuntimeError(f"unknown palette_format: {flavor.palette_format}")

    events, rc = run_mcmap_json(args)
    if rc != 0:
        raise RuntimeError(
            f"gen-palette {flavor.palette_format} failed for {flavor.id}: {events!r}"
        )
    assert_result(events)
    return out


def palette_lookup(palette_path: Path, key: str) -> tuple[int, int, int, int] | None:
    """Look up a block's RGBA from a generated palette JSON.

    Modern palettes are flat `{"ns:name": [r,g,b,a]}`. Legacy/forge112 are
    `{"format": ..., "blocks": {"id|meta": [r,g,b,a]}}` keyed by the FML
    numeric id. For legacy/forge112 the caller must pre-resolve the
    `minecraft:<name>` to its `id|meta` key via the level.dat — that's
    out of scope for this helper, which only handles modern.
    """
    import json
    data = json.loads(palette_path.read_text())
    if isinstance(data, dict) and data.get("format") in ("1.7.10", "1.12.2"):
        return None  # caller must resolve numeric key
    rgba = data.get(key)
    if rgba is None:
        return None
    return tuple(int(x) for x in rgba)
