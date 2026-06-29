"""End-to-end tests for `prune-inhabited` with a real connected client."""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from asserts import assert_result, overworld_region_dir, run_mcmap_json
from cache import java_major_for, resolve_latest_release
from flavors import Flavor
from mcclient import MinecraftClient
from server import ServerInstance


BOT = "McmapBot"
Y = 70
THRESHOLD = 40
HIGH_WAIT_TICKS = 600
LOAD_WAIT_TICKS = 40
LEGACY_THRESHOLD = 200
LEGACY_LOW_WAIT_TICKS = 60

PLAYER_CHUNK = (0, 0)
DISTANT_CHUNK = (20, 0)
LOW_REGION_CHUNK = (40, 0)
DIMENSION_FOLDERS = [
    Path("DIM-1"),
    Path("dimensions") / "mcmap" / "test",
]

LOW_BLOCK = "minecraft:diamond_block"
HIGH_BLOCK = "minecraft:gold_block"
CHECK_BLOCK = "minecraft:lapis_block"


MODERN_PRUNE_VERSIONS = [
    "1.13.2",
    "1.14.4",
    "1.15.2",
    "1.16.5",
    "1.17.1",
    "1.18.2",
    "1.19.4",
    "1.20.6",
    "1.21.1",
    "1.21.8",
    "latest",
]
LEGACY_PRUNE_VERSIONS = ["1.7.10", "1.12.2"]


def _vanilla_prune_flavor(version: str) -> Flavor:
    if version == "latest":
        mc_version = resolve_latest_release()
        return Flavor(
            id="vanilla-latest",
            distribution="vanilla",
            mc_version=mc_version,
            java_major=java_major_for(mc_version),
            level_type="minecraft:flat",
        )
    legacy = version in {"1.7.10", "1.12.2"}
    return Flavor(
        id=f"vanilla-{version}",
        distribution="vanilla",
        mc_version=version,
        java_major=java_major_for(version),
        level_type="FLAT" if legacy else "minecraft:flat",
        generator_settings="3;7,2*3,2;1" if legacy else "",
    )


MODERN_PRUNE_FLAVORS = [_vanilla_prune_flavor(v) for v in MODERN_PRUNE_VERSIONS]
LEGACY_PRUNE_FLAVORS = [_vanilla_prune_flavor(v) for v in LEGACY_PRUNE_VERSIONS]
ALL_PRUNE_FLAVORS = MODERN_PRUNE_FLAVORS + LEGACY_PRUNE_FLAVORS


def _work_dir_for(request: pytest.FixtureRequest, run_root: Path, flavor: Flavor) -> Path:
    safe_name = request.node.name.replace("/", "_").replace("[", "_").replace("]", "_")
    path = run_root / flavor.id / safe_name
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    return path


@pytest.fixture(params=MODERN_PRUNE_FLAVORS, ids=lambda f: f.id)
def modern_prune_flavor(request: pytest.FixtureRequest) -> Flavor:
    return request.param


@pytest.fixture
def modern_work_dir(
    request: pytest.FixtureRequest,
    run_root: Path,
    modern_prune_flavor: Flavor,
) -> Path:
    return _work_dir_for(request, run_root, modern_prune_flavor)


@pytest.fixture(params=LEGACY_PRUNE_FLAVORS, ids=lambda f: f.id)
def legacy_prune_flavor(request: pytest.FixtureRequest) -> Flavor:
    return request.param


@pytest.fixture
def legacy_work_dir(
    request: pytest.FixtureRequest,
    run_root: Path,
    legacy_prune_flavor: Flavor,
) -> Path:
    return _work_dir_for(request, run_root, legacy_prune_flavor)


@pytest.fixture(params=ALL_PRUNE_FLAVORS, ids=lambda f: f.id)
def prune_flavor(request: pytest.FixtureRequest) -> Flavor:
    return request.param


@pytest.fixture
def prune_work_dir(
    request: pytest.FixtureRequest,
    run_root: Path,
    prune_flavor: Flavor,
) -> Path:
    return _work_dir_for(request, run_root, prune_flavor)


def _marker_pos(chunk: tuple[int, int]) -> tuple[int, int, int]:
    cx, cz = chunk
    return cx * 16 + 1, Y, cz * 16 + 1


def _chunk_center(chunk: tuple[int, int]) -> tuple[int, int, int]:
    cx, cz = chunk
    return cx * 16 + 8, Y + 5, cz * 16 + 8


def _region_of(chunk: tuple[int, int]) -> tuple[int, int]:
    cx, cz = chunk
    return cx // 32, cz // 32


def _load_with_player(srv: ServerInstance, chunk: tuple[int, int]) -> None:
    x, y, z = _chunk_center(chunk)
    srv.teleport_player(BOT, x, y, z)
    srv.wait_ticks(LOAD_WAIT_TICKS)


def _place_loaded_marker(
    srv: ServerInstance,
    chunk: tuple[int, int],
    block: str,
) -> None:
    x, y, z = _marker_pos(chunk)
    srv.setblock(x, y, z, block)


def _region_file_for_chunk(work_dir: Path, chunk: tuple[int, int]) -> Path:
    rx, rz = _region_of(chunk)
    region_file = overworld_region_dir(work_dir) / f"r.{rx}.{rz}.mca"
    assert region_file.exists(), f"no region file at {region_file}"
    return region_file


def _copy_region_to_dimensions(work_dir: Path, region_file: Path) -> list[Path]:
    copied = []
    for folder in DIMENSION_FOLDERS:
        target_dir = work_dir / "world" / folder / "region"
        target_dir.mkdir(parents=True, exist_ok=True)
        target = target_dir / region_file.name
        shutil.copy2(region_file, target)
        copied.append(target)
    return copied


def _assert_region_files_match(reference: Path, copies: list[Path]) -> None:
    expected = reference.read_bytes()
    for copy in copies:
        assert copy.read_bytes() == expected, (
            f"copied dimension region {copy} differs from {reference}"
        )


def _pruned_chunks(events: list[dict]) -> set[tuple[int, int]]:
    return {
        (int(ev["chunk_x"]), int(ev["chunk_z"]))
        for ev in events
        if ev.get("type") == "chunk_pruned"
    }


def _pruned_chunk_events(
    events: list[dict],
    chunk: tuple[int, int],
) -> list[dict]:
    cx, cz = chunk
    return [
        ev
        for ev in events
        if ev.get("type") == "chunk_pruned"
        and int(ev["chunk_x"]) == cx
        and int(ev["chunk_z"]) == cz
    ]


def _pruned_regions(events: list[dict]) -> set[tuple[int, int]]:
    return {
        (int(ev["region_x"]), int(ev["region_z"]))
        for ev in events
        if ev.get("type") == "region_pruned"
    }


def _assert_prune_progress(events: list[dict], result: dict, *, dry_run: bool) -> None:
    phase = "scan" if dry_run else "prune"
    progress = [
        (i, ev)
        for i, ev in enumerate(events)
        if ev.get("type") == "progress"
    ]
    assert progress, f"no progress events emitted: {events!r}"
    assert all(ev.get("phase") == phase for _, ev in progress), (
        f"unexpected prune progress phase: {progress!r}"
    )
    totals = {int(ev["regions_total"]) for _, ev in progress}
    assert totals == {int(result["regions_scanned"])}, (
        f"progress totals do not match result: {progress!r} vs {result!r}"
    )
    processed = [int(ev["regions_processed"]) for _, ev in progress]
    assert processed == list(range(1, int(result["regions_scanned"]) + 1)), (
        f"progress processed counts are not monotonic: {progress!r}"
    )
    assert progress[-1][0] == len(events) - 2, (
        f"final progress event should immediately precede result: {events!r}"
    )

    prune_event_indexes = [
        i
        for i, ev in enumerate(events)
        if ev.get("type") in {"chunk_pruned", "region_pruned"}
    ]
    if prune_event_indexes:
        assert min(prune_event_indexes) < progress[-1][0], (
            f"prune events were delayed until after scanning: {events!r}"
        )


def _event_region_path(event: dict) -> str:
    return str(event["region"]).replace("\\", "/")


def _verify_dimension_prunes(events: list[dict], chunk: tuple[int, int]) -> None:
    region_paths = [_event_region_path(ev) for ev in _pruned_chunk_events(events, chunk)]
    assert any(
        "/world/region/" in path or "/world/dimensions/minecraft/overworld/" in path
        for path in region_paths
    ), (
        f"overworld chunk {chunk} was not pruned: {events!r}"
    )
    for folder in DIMENSION_FOLDERS:
        expected = f"/{folder.as_posix()}/region/"
        assert any(expected in path for path in region_paths), (
            f"dimension chunk {chunk} under {folder.as_posix()} was not pruned: {events!r}"
        )


def _verify_marker(
    srv: ServerInstance,
    chunk: tuple[int, int],
    block: str,
    *,
    present: bool,
) -> None:
    _load_with_player(srv, chunk)
    x, y, z = _marker_pos(chunk)
    if present:
        srv.assert_block(x, y, z, block)
    else:
        assert not srv.block_at_is(x, y, z, block), (
            f"chunk {chunk} still has marker {block} at {(x, y, z)}"
        )


def _verify_writable(srv: ServerInstance, chunk: tuple[int, int]) -> None:
    _load_with_player(srv, chunk)
    x, y, z = _marker_pos(chunk)
    srv.setblock(x, y, z, CHECK_BLOCK)
    srv.assert_block(x, y, z, CHECK_BLOCK)


def _run_prune(
    work_dir: Path,
    *,
    threshold: int = THRESHOLD,
    mode: str = "chunks",
    dry_run: bool = False,
) -> list[dict]:
    args = [
        "prune-inhabited",
        str(work_dir / "world"),
        "--threshold",
        str(threshold),
        "--mode",
        mode,
    ]
    if dry_run:
        args.append("--dry-run")
    events, rc = run_mcmap_json(args)
    assert rc == 0, f"prune-inhabited failed: {events!r}"
    result = assert_result(events)
    assert result["mode"] == mode
    assert result["dry_run"] is dry_run
    _assert_prune_progress(events, result, dry_run=dry_run)
    return events


def test_prune_inhabited_keeps_player_chunk_and_prunes_distant_chunk(
    modern_work_dir: Path,
    modern_prune_flavor: Flavor,
) -> None:
    with ServerInstance(modern_prune_flavor, modern_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _place_loaded_marker(srv, DISTANT_CHUNK, LOW_BLOCK)
            _load_with_player(srv, PLAYER_CHUNK)
            _place_loaded_marker(srv, PLAYER_CHUNK, HIGH_BLOCK)
            srv.wait_ticks(HIGH_WAIT_TICKS)

    region_file = _region_file_for_chunk(modern_work_dir, DISTANT_CHUNK)
    dimension_regions = _copy_region_to_dimensions(modern_work_dir, region_file)
    _assert_region_files_match(region_file, dimension_regions)

    events = _run_prune(modern_work_dir)
    _assert_region_files_match(region_file, dimension_regions)
    pruned = _pruned_chunks(events)
    assert DISTANT_CHUNK in pruned, f"distant chunk was not pruned: {events!r}"
    assert PLAYER_CHUNK not in pruned, (
        f"player chunk was unexpectedly pruned: {events!r}"
    )
    _verify_dimension_prunes(events, DISTANT_CHUNK)

    with ServerInstance(modern_prune_flavor, modern_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _verify_marker(srv, DISTANT_CHUNK, LOW_BLOCK, present=False)
            _verify_marker(srv, PLAYER_CHUNK, HIGH_BLOCK, present=True)
            for chunk in [DISTANT_CHUNK, PLAYER_CHUNK]:
                _verify_writable(srv, chunk)


def test_prune_inhabited_dry_run_keeps_world(
    modern_work_dir: Path,
    modern_prune_flavor: Flavor,
) -> None:
    with ServerInstance(modern_prune_flavor, modern_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _place_loaded_marker(srv, DISTANT_CHUNK, LOW_BLOCK)
            _load_with_player(srv, PLAYER_CHUNK)
            _place_loaded_marker(srv, PLAYER_CHUNK, HIGH_BLOCK)
            srv.wait_ticks(HIGH_WAIT_TICKS)

    events = _run_prune(modern_work_dir, dry_run=True)
    pruned = _pruned_chunks(events)
    assert DISTANT_CHUNK in pruned, (
        f"dry-run did not report distant chunk: {events!r}"
    )
    assert PLAYER_CHUNK not in pruned, (
        f"dry-run selected player chunk: {events!r}"
    )
    assert all(
        ev.get("dry_run") is True
        for ev in events
        if ev.get("type") == "chunk_pruned"
    )

    with ServerInstance(modern_prune_flavor, modern_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _verify_marker(srv, DISTANT_CHUNK, LOW_BLOCK, present=True)
            _verify_marker(srv, PLAYER_CHUNK, HIGH_BLOCK, present=True)


def test_prune_inhabited_region_mode_prunes_only_all_low_regions(
    prune_work_dir: Path,
    prune_flavor: Flavor,
) -> None:
    legacy = prune_flavor.mc_version in LEGACY_PRUNE_VERSIONS
    with ServerInstance(prune_flavor, prune_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            if legacy:
                _load_with_player(srv, LOW_REGION_CHUNK)
                srv.wait_ticks(LEGACY_LOW_WAIT_TICKS)
            _place_loaded_marker(srv, LOW_REGION_CHUNK, LOW_BLOCK)
            _load_with_player(srv, PLAYER_CHUNK)
            _place_loaded_marker(srv, PLAYER_CHUNK, HIGH_BLOCK)
            srv.wait_ticks(HIGH_WAIT_TICKS)

    threshold = LEGACY_THRESHOLD if legacy else THRESHOLD
    events = _run_prune(prune_work_dir, threshold=threshold, mode="regions")
    pruned = _pruned_regions(events)
    assert _region_of(LOW_REGION_CHUNK) in pruned, (
        f"low-only region was not pruned: {events!r}"
    )
    assert _region_of(PLAYER_CHUNK) not in pruned, (
        f"region containing high player chunk was pruned: {events!r}"
    )

    with ServerInstance(prune_flavor, prune_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _verify_marker(srv, LOW_REGION_CHUNK, LOW_BLOCK, present=False)
            _verify_marker(srv, PLAYER_CHUNK, HIGH_BLOCK, present=True)
            for chunk in [LOW_REGION_CHUNK, PLAYER_CHUNK]:
                _verify_writable(srv, chunk)


def test_prune_inhabited_legacy_keeps_player_chunk_and_prunes_distant_chunk(
    legacy_work_dir: Path,
    legacy_prune_flavor: Flavor,
) -> None:
    with ServerInstance(legacy_prune_flavor, legacy_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _place_loaded_marker(srv, PLAYER_CHUNK, HIGH_BLOCK)
            _load_with_player(srv, DISTANT_CHUNK)
            srv.wait_ticks(LEGACY_LOW_WAIT_TICKS)
            _place_loaded_marker(srv, DISTANT_CHUNK, LOW_BLOCK)
            _load_with_player(srv, PLAYER_CHUNK)
            srv.wait_ticks(HIGH_WAIT_TICKS)

    region_file = _region_file_for_chunk(legacy_work_dir, DISTANT_CHUNK)
    dimension_regions = _copy_region_to_dimensions(legacy_work_dir, region_file)
    _assert_region_files_match(region_file, dimension_regions)

    events = _run_prune(legacy_work_dir, threshold=LEGACY_THRESHOLD)
    _assert_region_files_match(region_file, dimension_regions)
    pruned = _pruned_chunks(events)
    assert DISTANT_CHUNK in pruned, f"legacy distant chunk was not pruned: {events!r}"
    assert PLAYER_CHUNK not in pruned, (
        f"legacy player chunk was unexpectedly pruned: {events!r}"
    )
    _verify_dimension_prunes(events, DISTANT_CHUNK)

    with ServerInstance(legacy_prune_flavor, legacy_work_dir) as srv:
        assert srv.port is not None
        with MinecraftClient("127.0.0.1", srv.port, BOT):
            srv.set_player_survival(BOT)
            _verify_marker(srv, DISTANT_CHUNK, LOW_BLOCK, present=False)
            _verify_marker(srv, PLAYER_CHUNK, HIGH_BLOCK, present=True)
            for chunk in [DISTANT_CHUNK, PLAYER_CHUNK]:
                _verify_writable(srv, chunk)
