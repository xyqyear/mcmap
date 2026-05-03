"""Test `gen-palette` against a real generated world for each flavor.

For modern flavors: pass the vanilla client jar; assert palette has many
entries, that several known blocks have non-default colors, and that those
colors are mutually distinguishable.

For legacy / forge112 flavors: bootstrap a base world (so level.dat exists),
then run gen-palette with that level.dat plus the matching client jar (and
mod jars for forge112). Assert palette is wrapped with the correct format
tag and that a few known FML-resolved id|meta keys are populated and
distinguishable.
"""

from __future__ import annotations

import json
from pathlib import Path

from asserts import assert_result, run_mcmap_json
from cache import vanilla_client_jar
from flavors import Flavor
from level_dat import legacy_block_registry, forge112_block_registry
from server import ServerInstance


KNOWN_NAMES = ("minecraft:stone", "minecraft:gold_block", "minecraft:diamond_block")


def _client_jar_for(flavor: Flavor, work_dir: Path) -> Path:
    dest = work_dir / f"client-{flavor.mc_version}.jar"
    return vanilla_client_jar(flavor.mc_version, dest)


def _max_channel_diff(a: list[int], b: list[int]) -> int:
    return max(abs(int(x) - int(y)) for x, y in zip(a[:3], b[:3]))


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
        # All three known names must be present with sane RGBA.
        rgba_by_name: dict[str, list[int]] = {}
        for name in KNOWN_NAMES:
            entry = palette.get(name)
            assert entry is not None, f"modern palette missing {name}"
            assert len(entry) == 4
            assert entry[3] == 255, f"{name} alpha != 255: {entry}"
            assert any(c > 0 for c in entry[:3]), f"{name} is all-zero RGB"
            rgba_by_name[name] = entry
        # gold vs diamond must differ visibly — they're different in vanilla.
        gold = rgba_by_name["minecraft:gold_block"]
        diamond = rgba_by_name["minecraft:diamond_block"]
        stone = rgba_by_name["minecraft:stone"]
        assert _max_channel_diff(gold, diamond) >= 30, (
            f"gold {gold} and diamond {diamond} are too close; sampler may be wrong"
        )
        assert _max_channel_diff(gold, stone) >= 30, (
            f"gold {gold} and stone {stone} are too close"
        )
        # Result `entries` should match the JSON object size exactly.
        assert result["entries"] == len(palette), (
            f"result.entries={result['entries']} but palette has {len(palette)} keys"
        )
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

    # Resolve known block names to their FML id|0 keys via level.dat. Both
    # registries use the same lookup shape; the Python helper module mirrors
    # what `gen_palette/{legacy,forge112}/leveldat.rs` does.
    if _flavor.palette_format == "legacy":
        registry = legacy_block_registry(level_dat)
    else:
        registry = forge112_block_registry(level_dat)

    rgba_by_name: dict[str, list[int]] = {}
    for name in KNOWN_NAMES:
        assert name in registry, (
            f"FML registry missing {name!r} — sample: {list(registry)[:10]}"
        )
        key = f"{registry[name]}|0"
        entry = blocks.get(key)
        assert entry is not None, (
            f"palette[{key!r}] (=={name}) missing in {_flavor.id} palette"
        )
        assert len(entry) == 4
        assert entry[3] == 255, f"{name} alpha != 255: {entry}"
        rgba_by_name[name] = entry

    gold = rgba_by_name["minecraft:gold_block"]
    diamond = rgba_by_name["minecraft:diamond_block"]
    stone = rgba_by_name["minecraft:stone"]
    assert _max_channel_diff(gold, diamond) >= 30, (
        f"{_flavor.id}: gold {gold} and diamond {diamond} too close"
    )
    assert _max_channel_diff(gold, stone) >= 30, (
        f"{_flavor.id}: gold {gold} and stone {stone} too close"
    )


def test_gen_palette_overrides_take_precedence(work_dir: Path, _flavor: Flavor) -> None:
    """A user override file must replace the auto-resolved color for the
    listed key. Round-trips through every gen-palette subcommand because
    overrides are handled in `shared/overrides.rs`.

    Use a wildly-distinct color (hot pink) so the assertion can't pass by
    coincidence.
    """
    out = work_dir / "palette.json"
    overrides = work_dir / "overrides.json"
    pink = [255, 20, 147, 255]

    if _flavor.palette_format == "modern":
        # Modern overrides are keyed by `ns:name`.
        overrides.write_text(json.dumps({"minecraft:gold_block": pink}))
        client_jar = _client_jar_for(_flavor, work_dir)
        events, rc = run_mcmap_json([
            "gen-palette", "modern",
            "-p", str(client_jar),
            "-o", str(out),
            "--overrides", str(overrides),
        ])
        assert rc == 0, f"gen-palette modern failed: {events!r}"
        assert_result(events)
        palette = json.loads(out.read_text())
        assert palette["minecraft:gold_block"] == pink, (
            f"override not applied: got {palette.get('minecraft:gold_block')!r}"
        )
        return

    # Legacy/forge112: overrides are keyed by `id` or `id|meta`. Resolve via
    # FML registry, key the override by `id|0`.
    with ServerInstance(_flavor, work_dir):
        pass
    level_dat = work_dir / "world" / "level.dat"
    if _flavor.palette_format == "legacy":
        reg = legacy_block_registry(level_dat)
    else:
        reg = forge112_block_registry(level_dat)
    gold_key = f"{reg['minecraft:gold_block']}|0"
    overrides.write_text(json.dumps({gold_key: pink}))

    client_jar = _client_jar_for(_flavor, work_dir)
    args = [
        "gen-palette", _flavor.palette_format,
        "--level-dat", str(level_dat),
        "--pack", str(client_jar),
        "-o", str(out),
        "--overrides", str(overrides),
    ]
    if _flavor.palette_format == "forge112":
        mods_dir = work_dir / "mods"
        for jar in sorted(mods_dir.glob("*.jar")):
            args.extend(["--pack", str(jar)])

    events, rc = run_mcmap_json(args)
    assert rc == 0, f"gen-palette failed: {events!r}"
    assert_result(events)
    palette = json.loads(out.read_text())
    blocks = palette["blocks"]
    assert blocks.get(gold_key) == pink, (
        f"override not applied to {gold_key}: got {blocks.get(gold_key)!r}"
    )
