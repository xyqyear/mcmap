"""End-to-end test for `remove-chunks`.

Place block X in chunk K, save+stop. Run remove-chunks to empty K. Re-boot
the server: the chunk regenerates from the world generator, so the placed
block must no longer be there.

Verification of untouched chunks is also done via in-game commands rather
than byte-identical checks. Byte equality after remove-chunks would only
prove the file was untouched on disk — but if mcmap had subtly corrupted
the .mca, the server might silently regenerate that chunk on the next
boot and the byte-check would still pass while the user-observable world
had changed. In-game verification reflects what the server actually read.
"""

from __future__ import annotations

from pathlib import Path

from asserts import assert_error, assert_result, find_event, overworld_region_dir, run_mcmap_json
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
    assert not chunk_event.get("was_empty"), (
        f"unexpected was_empty=true on populated target: {chunk_event!r}"
    )

    # Re-boot. The chunk is now empty on disk; the server will regenerate it.
    # Whatever it regenerates to, it isn't the placed block.
    with ServerInstance(_flavor, work_dir) as srv:
        assert not srv.block_at_is(TARGET_X, TARGET_Y, TARGET_Z, PLACED), (
            "remove-chunks did not remove the placed block — chunk still has it"
        )


# Same 4-chunk grid as test_replace_chunks: (0,0), (1,0), (0,1), (1,1) inside
# region (0, 0). All four are inside the legacy spawn-chunks square and
# modern force-loads each one explicitly on /setblock.
MULTI_CHUNKS: list[tuple[int, int, int, int, int]] = [
    (0, 0, 0, 70, 0),
    (1, 0, 16, 70, 0),
    (0, 1, 0, 70, 16),
    (1, 1, 16, 70, 16),
]


def test_remove_chunks_multi_preserves_untouched(
    work_dir: Path, _flavor: Flavor
) -> None:
    """Place a marker in 4 chunks, remove-chunks 2 of them. Re-boot and
    verify in-game: the 2 removed chunks no longer have the marker (they
    regenerated), the 2 untouched chunks still do.
    """
    with ServerInstance(_flavor, work_dir) as srv:
        for (_cx, _cz, x, y, z) in MULTI_CHUNKS:
            srv.setblock(x, y, z, PLACED)
            srv.assert_block(x, y, z, PLACED)
    region_dir = overworld_region_dir(work_dir)
    region_file = region_dir / "r.0.0.mca"

    to_remove = MULTI_CHUNKS[:2]
    untouched = MULTI_CHUNKS[2:]
    chunks_arg = ";".join(f"{cx},{cz}" for (cx, cz, *_rest) in to_remove)
    events, rc = run_mcmap_json([
        "remove-chunks", "-t", str(region_file), "-c", chunks_arg,
    ])
    assert rc == 0, f"remove-chunks failed: {events!r}"
    result = assert_result(events)
    assert result["removed"] == 2

    with ServerInstance(_flavor, work_dir) as srv:
        for (_cx, _cz, x, y, z) in to_remove:
            assert not srv.block_at_is(x, y, z, PLACED), (
                f"chunk ({_cx},{_cz}) was supposed to be removed but still has marker at ({x},{y},{z})"
            )
        for (_cx, _cz, x, y, z) in untouched:
            srv.assert_block(x, y, z, PLACED)


def test_remove_chunks_placeholder_target_no_op(work_dir: Path, _flavor: Flavor) -> None:
    """A 0-byte vanilla placeholder target should stay 0 bytes after a
    remove-chunks call (no-op). Event should mark `was_empty=true`.
    """
    placeholder = work_dir / "r.0.0.mca"
    placeholder.write_bytes(b"")
    assert placeholder.stat().st_size == 0
    events, rc = run_mcmap_json([
        "remove-chunks", "-t", str(placeholder), "-c", "0,0;1,1",
    ])
    assert rc == 0, f"remove-chunks failed: {events!r}"
    result = assert_result(events)
    assert result["removed"] == 2
    for (cx, cz) in [(0, 0), (1, 1)]:
        ev = find_event(events, type="chunk_removed", x=cx, z=cz)
        assert ev is not None
        assert ev.get("was_empty") is True, (
            f"expected was_empty=true on placeholder, got {ev!r}"
        )
    # File must still be 0 bytes — vanilla relies on this for its poi/
    # placeholder behavior.
    assert placeholder.stat().st_size == 0, "placeholder was promoted to non-zero"


# --- Error paths -----------------------------------------------------------


def test_remove_chunks_missing_target_rejected(work_dir: Path, _flavor: Flavor) -> None:
    events, rc = run_mcmap_json([
        "remove-chunks", "-t", str(work_dir / "no-such.mca"), "-c", "0,0",
    ])
    assert rc != 0
    assert_error(events)


def test_remove_chunks_out_of_range_coord_rejected(work_dir: Path, _flavor: Flavor) -> None:
    target = work_dir / "r.0.0.mca"
    target.write_bytes(b"\x00" * 8192)
    events, rc = run_mcmap_json([
        "remove-chunks", "-t", str(target), "-c", "0,32",
    ])
    assert rc != 0
    err = assert_error(events)
    assert "32" in err["message"] or "range" in err["message"].lower(), (
        f"unexpected error message: {err!r}"
    )
