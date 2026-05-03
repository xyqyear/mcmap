"""End-to-end test for `replace-chunks`.

Recipe: place block A in chunk K, snapshot the region file as backup. Place
block B at the same spot (overwriting A), save+stop. Run replace-chunks to
copy K from backup into the live world. Re-boot the server and use
testforblock/execute to verify the live world now reads back as block A.
"""

from __future__ import annotations

import shutil
from pathlib import Path

from asserts import assert_result, find_event, overworld_region_dir, run_mcmap_json
from flavors import Flavor
from server import ServerInstance


# Coordinates that fall inside region (0, 0), chunk (4, 4) (= relative slot
# (4, 4)). World coords for a block in chunk (4, 4) at chunk-local (0, 0):
# (4*16 + 0, _, 4*16 + 0) = (64, 70, 64).
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
