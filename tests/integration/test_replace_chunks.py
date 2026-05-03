"""End-to-end test for `replace-chunks`.

Recipe: place block A in chunk K, snapshot the region file as backup. Place
block B at the same spot (overwriting A), save+stop. Run replace-chunks to
copy K from backup into the live world. Re-boot the server and use
testforblock/execute to verify the live world now reads back as block A.

Verification of untouched chunks is also done via in-game commands rather
than byte-identical checks. Byte equality after replace-chunks would only
prove the file was untouched on disk — but if mcmap had subtly corrupted
the .mca, a vanilla server might silently regenerate that chunk on the
next boot and the byte-check would still pass while the user-observable
world had changed. In-game verification reflects what the server actually
read out of the file.
"""

from __future__ import annotations

import shutil
from pathlib import Path

from asserts import assert_error, assert_result, find_event, overworld_region_dir, run_mcmap_json
from flavors import Flavor
from server import ServerInstance


# Single-chunk round-trip: chunk (4, 4), block at chunk-local (0, 0) ->
# world (64, 70, 64). Inside region (0, 0).
TARGET_X, TARGET_Y, TARGET_Z = 64, 70, 64
REL_CX, REL_CZ = 4, 4
BLOCK_A = "minecraft:gold_block"
BLOCK_B = "minecraft:diamond_block"


def test_replace_chunks_round_trip(work_dir: Path, _flavor: Flavor) -> None:
    # Phase 1: place block A, save+stop, snapshot the region file.
    with ServerInstance(_flavor, work_dir) as srv:
        srv.setblock(TARGET_X, TARGET_Y, TARGET_Z, BLOCK_A)
    region_dir = overworld_region_dir(work_dir)
    region_file = region_dir / "r.0.0.mca"
    assert region_file.exists(), f"no region file at {region_file}"
    backup_dir = work_dir / "backup"
    backup_dir.mkdir()
    backup_file = backup_dir / "r.0.0.mca"
    shutil.copy2(region_file, backup_file)

    # Phase 2: overwrite with block B.
    with ServerInstance(_flavor, work_dir) as srv:
        srv.setblock(TARGET_X, TARGET_Y, TARGET_Z, BLOCK_B)

    # Phase 3: replace-chunks to restore K from the backup.
    events, rc = run_mcmap_json([
        "replace-chunks",
        "-s", str(backup_file),
        "-t", str(region_file),
        "-c", f"{REL_CX},{REL_CZ}",
    ])
    assert rc == 0, f"replace-chunks failed: {events!r}"
    result = assert_result(events)
    assert result["replaced"] == 1
    chunk_event = find_event(events, type="chunk_replaced", x=REL_CX, z=REL_CZ)
    assert chunk_event is not None, f"no chunk_replaced event: {events!r}"
    assert chunk_event["source_kind"] == "inline", (
        f"unexpected source_kind: {chunk_event!r}"
    )

    # Phase 4: re-boot, verify the live world now reads as block A.
    with ServerInstance(_flavor, work_dir) as srv:
        srv.assert_block(TARGET_X, TARGET_Y, TARGET_Z, BLOCK_A)


# Multi-chunk: place markers in 4 chunks within region (0, 0). Their world
# coords stay near the spawn-loaded origin so legacy /setblock works without
# extra forceload tickets. Y stays at 70 so we don't fight chunk-local
# heightmap quirks.
#
# All four are inside the legacy spawn-chunks square (radius-8 around chunk
# (0,0)), and modern force-loads each one explicitly on /setblock.
MULTI_CHUNKS: list[tuple[int, int, int, int, int]] = [
    # (rel_cx, rel_cz, world_x, world_y, world_z)
    (0, 0, 0, 70, 0),
    (1, 0, 16, 70, 0),
    (0, 1, 0, 70, 16),
    (1, 1, 16, 70, 16),
]


def test_replace_chunks_multi_preserves_untouched(work_dir: Path, _flavor: Flavor) -> None:
    """Place block A in 4 chunks, snapshot, overwrite all 4 with B, then
    replace half (2 of 4) from backup. Verify in-game:

      - the 2 replaced chunks read back as A
      - the 2 untouched chunks still read as B

    This catches both "did the replace target the right slots" and "did the
    operation accidentally clobber adjacent slots".
    """
    # Phase 1: place A everywhere, snapshot.
    with ServerInstance(_flavor, work_dir) as srv:
        for (_cx, _cz, x, y, z) in MULTI_CHUNKS:
            srv.setblock(x, y, z, BLOCK_A)
    region_dir = overworld_region_dir(work_dir)
    region_file = region_dir / "r.0.0.mca"
    backup = work_dir / "backup" / "r.0.0.mca"
    backup.parent.mkdir()
    shutil.copy2(region_file, backup)

    # Phase 2: overwrite all four with B.
    with ServerInstance(_flavor, work_dir) as srv:
        for (_cx, _cz, x, y, z) in MULTI_CHUNKS:
            srv.setblock(x, y, z, BLOCK_B)

    # Phase 3: replace-chunks for the first two (rel chunks (0,0) and (1,0)).
    to_replace = MULTI_CHUNKS[:2]
    untouched = MULTI_CHUNKS[2:]
    chunks_arg = ";".join(f"{cx},{cz}" for (cx, cz, *_rest) in to_replace)
    events, rc = run_mcmap_json([
        "replace-chunks",
        "-s", str(backup), "-t", str(region_file),
        "-c", chunks_arg,
    ])
    assert rc == 0, f"replace-chunks failed: {events!r}"
    result = assert_result(events)
    assert result["replaced"] == 2
    for (cx, cz, *_rest) in to_replace:
        ev = find_event(events, type="chunk_replaced", x=cx, z=cz)
        assert ev is not None, f"missing chunk_replaced for ({cx},{cz})"
        assert ev["source_kind"] == "inline"

    # Phase 4: re-boot. Replaced -> A; untouched -> B.
    with ServerInstance(_flavor, work_dir) as srv:
        for (_cx, _cz, x, y, z) in to_replace:
            srv.assert_block(x, y, z, BLOCK_A)
        for (_cx, _cz, x, y, z) in untouched:
            srv.assert_block(x, y, z, BLOCK_B)


def test_replace_chunks_empty_source_slot_clears_target(
    work_dir: Path, _flavor: Flavor
) -> None:
    """If the source's named slot is empty, the target's slot becomes empty.

    Recipe: place block in chunk (4, 4), save. Snapshot. Run remove-chunks on
    the snapshot to empty slot (4, 4). Now use that emptied snapshot as the
    source for replace-chunks against the live target. Re-boot: the chunk
    will have been regenerated by the worldgen, so the placed block must
    no longer be there.
    """
    with ServerInstance(_flavor, work_dir) as srv:
        srv.setblock(TARGET_X, TARGET_Y, TARGET_Z, BLOCK_A)
    region_dir = overworld_region_dir(work_dir)
    region_file = region_dir / "r.0.0.mca"
    backup = work_dir / "backup" / "r.0.0.mca"
    backup.parent.mkdir()
    shutil.copy2(region_file, backup)

    # Hollow the backup at slot (4, 4) so it has an empty slot there.
    events, rc = run_mcmap_json([
        "remove-chunks", "-t", str(backup), "-c", f"{REL_CX},{REL_CZ}",
    ])
    assert rc == 0, f"setup remove-chunks failed: {events!r}"
    assert_result(events)

    # Replace target's chunk (4, 4) from the now-empty source slot.
    events, rc = run_mcmap_json([
        "replace-chunks", "-s", str(backup), "-t", str(region_file),
        "-c", f"{REL_CX},{REL_CZ}",
    ])
    assert rc == 0, f"replace-chunks failed: {events!r}"
    chunk_event = find_event(events, type="chunk_replaced", x=REL_CX, z=REL_CZ)
    assert chunk_event is not None
    assert chunk_event["source_kind"] == "empty", (
        f"expected source_kind=empty, got {chunk_event!r}"
    )

    # Re-boot: chunk regenerates from worldgen. Placed block is gone.
    with ServerInstance(_flavor, work_dir) as srv:
        assert not srv.block_at_is(TARGET_X, TARGET_Y, TARGET_Z, BLOCK_A), (
            "replace-chunks from empty source did not clear the chunk"
        )


# --- Error paths -----------------------------------------------------------


def test_replace_chunks_identical_paths_rejected(work_dir: Path, _flavor: Flavor) -> None:
    """`-s` and `-t` pointing to the same path → type=error."""
    region_file = work_dir / "r.0.0.mca"
    region_file.write_bytes(b"\x00" * (8192))  # any 8-KB header is fine; we never reach apply
    events, rc = run_mcmap_json([
        "replace-chunks", "-s", str(region_file), "-t", str(region_file),
        "-c", "0,0",
    ])
    assert rc != 0
    err = assert_error(events)
    assert "same" in err["message"].lower(), f"unexpected error message: {err!r}"


def test_replace_chunks_out_of_range_coord_rejected(work_dir: Path, _flavor: Flavor) -> None:
    """Coord ≥ 32 → type=error before any I/O happens."""
    src = work_dir / "src.mca"
    tgt = work_dir / "r.0.0.mca"
    src.write_bytes(b"\x00" * 8192)
    tgt.write_bytes(b"\x00" * 8192)
    events, rc = run_mcmap_json([
        "replace-chunks", "-s", str(src), "-t", str(tgt),
        "-c", "32,0",
    ])
    assert rc != 0
    err = assert_error(events)
    assert "32" in err["message"] or "range" in err["message"].lower(), (
        f"unexpected error message: {err!r}"
    )


def test_replace_chunks_missing_source_rejected(work_dir: Path, _flavor: Flavor) -> None:
    """Non-existent source path → type=error."""
    tgt = work_dir / "r.0.0.mca"
    tgt.write_bytes(b"\x00" * 8192)
    events, rc = run_mcmap_json([
        "replace-chunks", "-s", str(work_dir / "no-such.mca"),
        "-t", str(tgt), "-c", "0,0",
    ])
    assert rc != 0
    assert_error(events)
