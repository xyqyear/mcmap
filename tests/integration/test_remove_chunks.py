"""End-to-end test for `remove-chunks`.

Place block X in chunk K, save+stop. Run remove-chunks to empty K. Re-boot
the server: the chunk regenerates from the world generator, so the placed
block must no longer be there.
"""

from __future__ import annotations

from pathlib import Path

from asserts import assert_result, find_event, overworld_region_dir, run_mcmap_json
from flavors import Flavor
from server import ServerInstance


TARGET_X, TARGET_Y, TARGET_Z = 64, 70, 64
REL_CX, REL_CZ = 4, 4
PLACED = "minecraft:gold_block"


def test_remove_chunks_round_trip(work_dir: Path, _flavor: Flavor) -> None:
    with ServerInstance(_flavor, work_dir) as srv:
        srv.setblock(TARGET_X, TARGET_Y, TARGET_Z, PLACED)
        # Sanity-check inside the same boot: the block we just placed should
        # be readable. Catches /setblock typos before the test would falsely
        # pass via remove-chunks.
        srv.assert_block(TARGET_X, TARGET_Y, TARGET_Z, PLACED)
    region_dir = overworld_region_dir(work_dir)
    region_file = region_dir / "r.0.0.mca"
    assert region_file.exists(), f"no region file at {region_file}"

    events, rc = run_mcmap_json([
        "remove-chunks",
        "-t", str(region_file),
        "-c", f"{REL_CX},{REL_CZ}",
    ])
    assert rc == 0, f"remove-chunks failed: {events!r}"
    result = assert_result(events)
    assert result["removed"] == 1
    chunk_event = find_event(events, type="chunk_removed", x=REL_CX, z=REL_CZ)
    assert chunk_event is not None, f"no chunk_removed event: {events!r}"

    # Re-boot. The chunk is now empty on disk; the server will regenerate it.
    # Whatever it regenerates to, it isn't the placed block.
    with ServerInstance(_flavor, work_dir) as srv:
        assert not srv.block_at_is(TARGET_X, TARGET_Y, TARGET_Z, PLACED), (
            "remove-chunks did not remove the placed block — chunk still has it"
        )
