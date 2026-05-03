"""Read FML block-id registries from a server's level.dat.

mcmap's legacy/forge112 palette JSON is keyed by `id|meta` (1.7.10) or `id`
(Forge 1.12.2 with REI/JEID), where `id` is the per-world numeric id Forge
hands out at world-create time. The integration tests need to look up those
keys to assert palette colors and verify rendered pixels — same way the Rust
side does in `gen_palette/{legacy,forge112}/leveldat.rs`.

Two formats:

  - 1.7.10: `FML.ItemData` is a list of `{K: "<prefix><name>", V: int_id}`
    where the prefix byte is `\x01` for blocks and `\x02` for items.
  - 1.12.2: `FML.Registries.minecraft:blocks.ids` is a list of
    `{K: "ns:name", V: int_id}` (no prefix byte; uncapped under REI).

Both files are gzip-compressed NBT.
"""

from __future__ import annotations

from pathlib import Path

import nbtlib


def _root(level_dat: Path) -> nbtlib.Compound:
    """Return the unnamed root compound of a level.dat (gzip + NBT)."""
    nbt_file = nbtlib.load(str(level_dat), gzipped=True)
    # nbtlib.File is a Compound. The actual top-level data sits under one
    # named entry — for vanilla/Forge that's `""` (the unnamed wrapper) on
    # newer versions, but historically it's been keyed by an empty string
    # or by `Data`. Vanilla level.dat has a top-level Compound containing a
    # `Data` entry; Forge wraps the same under FML/Data alongside.
    return nbt_file


def legacy_block_registry(level_dat: Path) -> dict[str, int]:
    """1.7.10: return `{namespace:name -> numeric_id}` from FML.ItemData."""
    root = _root(level_dat)
    fml = root.get("FML")
    if fml is None:
        raise RuntimeError(f"no FML compound in {level_dat}")
    item_data = fml.get("ItemData")
    if item_data is None:
        raise RuntimeError(
            f"no FML.ItemData in {level_dat} — is this a Forge 1.7.10 world?"
        )

    blocks: dict[str, int] = {}
    for entry in item_data:
        k = str(entry.get("K", ""))
        v = int(entry.get("V", -1))
        if not k or v < 0:
            continue
        # Prefix byte 0x01 is blocks, 0x02 items. Strip the prefix.
        first = k[0]
        if first == "\x01":
            blocks[k[1:]] = v
        # 0x02 / others: items, skipped.
    return blocks


def forge112_block_registry(level_dat: Path) -> dict[str, int]:
    """1.12.2: return `{namespace:name -> numeric_id}` from FML.Registries."""
    root = _root(level_dat)
    fml = root.get("FML")
    if fml is None:
        raise RuntimeError(f"no FML compound in {level_dat}")
    registries = fml.get("Registries")
    if registries is None:
        raise RuntimeError(f"no FML.Registries in {level_dat}")
    blocks_reg = registries.get("minecraft:blocks")
    if blocks_reg is None:
        raise RuntimeError(
            f"no FML.Registries.'minecraft:blocks' in {level_dat} — "
            f"is this a 1.12.2 Forge world?"
        )
    ids = blocks_reg.get("ids")
    if ids is None:
        raise RuntimeError("FML.Registries.minecraft:blocks has no `ids` list")

    blocks: dict[str, int] = {}
    for entry in ids:
        k = str(entry.get("K", ""))
        v = int(entry.get("V", -1))
        if not k or v < 0:
            continue
        blocks[k] = v
    return blocks


def palette_key_for_block(palette_format: str, level_dat: Path, name: str) -> str:
    """Resolve the palette JSON key for a given `minecraft:<name>` block.

    - modern → returns `name` itself (palettes are flat namespaced names).
    - legacy → returns `"<id>|0"` where id comes from FML.ItemData.
    - forge112 → returns `"<id>|0"` where id comes from FML.Registries.

    `meta=0` is sufficient for the kinds of blocks the tests place
    (gold_block, diamond_block, stone — all single-meta).
    """
    if palette_format == "modern":
        return name
    if palette_format == "legacy":
        reg = legacy_block_registry(level_dat)
    elif palette_format == "forge112":
        reg = forge112_block_registry(level_dat)
    else:
        raise ValueError(f"unknown palette_format: {palette_format!r}")
    if name not in reg:
        raise KeyError(
            f"{name!r} not in FML registry for {level_dat} — "
            f"available sample: {list(reg)[:10]}..."
        )
    return f"{reg[name]}|0"


def max_block_id(palette_format: str, level_dat: Path) -> int:
    """Return the highest block id found in the FML registry. Used to assert
    that the modded-id space (>= 4096 under NEID/REI) is actually populated.
    """
    if palette_format == "legacy":
        reg = legacy_block_registry(level_dat)
    elif palette_format == "forge112":
        reg = forge112_block_registry(level_dat)
    else:
        raise ValueError(f"max_block_id only applies to legacy/forge112, got {palette_format!r}")
    return max(reg.values()) if reg else 0
