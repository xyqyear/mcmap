"""Test the `download-client` subcommand.

Doesn't need a server. Asserts mcmap reports the same sha1 as the manifest
and exercises the cache + alias + error paths.

When MCMAP_FLAVOR is set (CI matrix), this file runs only on the
`vanilla-latest` runner — there's no per-flavor split here, so running it
once is enough.
"""

from __future__ import annotations

import hashlib
import os
import tempfile
from pathlib import Path

import pytest

from asserts import assert_error, assert_result, find_event, run_mcmap_json
from cache import _per_version_manifest


pytestmark = pytest.mark.skipif(
    os.environ.get("MCMAP_FLAVOR") not in (None, "", "vanilla-latest"),
    reason="download-client tests run only on the vanilla-latest matrix runner",
)


@pytest.mark.parametrize("mc_version", ["1.7.10", "1.12.2"])
def test_download_client_legacy_versions(tmp_path: Path, mc_version: str) -> None:
    target = tmp_path / f"client-{mc_version}.jar"
    events, rc = run_mcmap_json([
        "download-client", mc_version, str(target),
    ])
    assert rc == 0, f"download-client failed: {events!r}"
    result = assert_result(events)
    assert result["version"] == mc_version
    assert result["bytes"] > 1_000_000

    # Compare against the manifest sha1 we resolve independently.
    info = _per_version_manifest(mc_version)
    expected_sha1 = info["downloads"]["client"]["sha1"]
    assert result["sha1"] == expected_sha1, (
        f"mcmap reported sha1 {result['sha1']}, manifest says {expected_sha1}"
    )

    # Cross-check by recomputing the file's actual sha1.
    h = hashlib.sha1()
    with target.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    assert h.hexdigest() == expected_sha1


def test_download_client_latest_alias(tmp_path: Path) -> None:
    """Test the `latest` alias resolves and downloads correctly."""
    target = tmp_path / "client-latest.jar"
    events, rc = run_mcmap_json([
        "download-client", "latest", str(target),
    ])
    assert rc == 0, f"download-client latest failed: {events!r}"
    result = assert_result(events)
    assert target.exists() and target.stat().st_size > 1_000_000
    assert result["sha1"]


def test_download_client_latest_snapshot_alias(tmp_path: Path) -> None:
    target = tmp_path / "client-snapshot.jar"
    events, rc = run_mcmap_json([
        "download-client", "latest-snapshot", str(target),
    ])
    assert rc == 0, f"download-client latest-snapshot failed: {events!r}"
    result = assert_result(events)
    assert target.exists() and target.stat().st_size > 1_000_000
    assert result["sha1"]
    # We can't predict the snapshot id, but the version_resolved event
    # should not be empty.
    resolved = find_event(events, type="progress", phase="version_resolved")
    assert resolved is not None and resolved["id"]


def test_download_client_unknown_version_emits_error(tmp_path: Path) -> None:
    """An unknown version id must emit type=error and exit non-zero."""
    target = tmp_path / "no.jar"
    events, rc = run_mcmap_json([
        "download-client", "9.9.9-not-a-real-version", str(target),
    ])
    assert rc != 0
    err = assert_error(events)
    assert "9.9.9-not-a-real-version" in err["message"] or "not found" in err["message"].lower()


def test_download_client_progress_monotonic_and_complete(tmp_path: Path) -> None:
    """The downloading-phase progress events must report non-decreasing
    `bytes` and the final tick must equal the manifest's reported size.

    The Rust code emits a final 100% tick independent of the throttle, so
    we don't tolerate falling-short on the last event.
    """
    target = tmp_path / "client-1.12.2.jar"
    events, rc = run_mcmap_json([
        "download-client", "1.12.2", str(target),
    ])
    assert rc == 0, f"download-client failed: {events!r}"
    result = assert_result(events)

    # Pull all downloading-phase progress events in order.
    download_events = [
        e for e in events
        if e.get("type") == "progress" and e.get("phase") == "downloading"
    ]
    if not download_events:
        # If a cached tmp file already had the correct sha1, the run skips
        # straight from cache_hit to verified — no downloading events. That
        # path is exercised by test_download_client_cache_hit_branch; skip
        # the monotonicity check here in that case.
        cache_hit = find_event(events, type="progress", phase="cache_hit")
        assert cache_hit is not None, (
            "no downloading events and no cache_hit either: " f"{events!r}"
        )
        return

    last_bytes = 0
    for ev in download_events:
        b = ev["bytes"]
        assert b >= last_bytes, f"bytes regressed: {last_bytes} -> {b}"
        assert ev["total"] == result["bytes"], (
            f"total {ev['total']} != result bytes {result['bytes']}"
        )
        last_bytes = b
    assert download_events[-1]["bytes"] == result["bytes"], (
        f"final downloading tick {download_events[-1]['bytes']} != total {result['bytes']}"
    )


def test_download_client_cache_hit_branch(tmp_path: Path) -> None:
    """The cache_hit branch fires when the system tmp file exists and its
    sha1 already matches the manifest. On the success path mcmap renames
    the tmp into the target — so two back-to-back runs don't reproduce
    cache_hit (the tmp is gone after the first run). To exercise it we
    download once, then plant the result back at the tmp path and invoke
    again. That mirrors the real-world recovery scenario where a previous
    invocation downloaded + verified but its move-into-target step failed.
    """
    import shutil

    # First run downloads cleanly. After this returns, the tmp at
    # /tmp/mcmap-client-1.12.2.jar.part is gone (renamed into target1).
    target1 = tmp_path / "first.jar"
    events_a, rc_a = run_mcmap_json([
        "download-client", "1.12.2", str(target1),
    ])
    assert rc_a == 0, f"first run failed: {events_a!r}"
    assert_result(events_a)

    # Plant the freshly-downloaded jar back at the tmp path so the next
    # invocation sees a valid cached tmp.
    tmp_loc = Path(tempfile.gettempdir()) / "mcmap-client-1.12.2.jar.part"
    shutil.copy2(target1, tmp_loc)
    try:
        target2 = tmp_path / "second.jar"
        events_b, rc_b = run_mcmap_json([
            "download-client", "1.12.2", str(target2),
        ])
        assert rc_b == 0, f"second run failed: {events_b!r}"
        assert_result(events_b)

        cache_hit = find_event(events_b, type="progress", phase="cache_hit")
        assert cache_hit is not None, (
            f"planted tmp did not trigger cache_hit: {events_b!r}"
        )
        # No downloading events expected on a cache hit.
        download_events = [
            e for e in events_b
            if e.get("type") == "progress" and e.get("phase") == "downloading"
        ]
        assert not download_events, (
            f"unexpected downloading events on cache hit: {download_events!r}"
        )
    finally:
        if tmp_loc.exists():
            tmp_loc.unlink()


def test_download_client_cache_miss_with_stale_tmp(tmp_path: Path) -> None:
    """If the system tmp file exists but has the wrong sha1, mcmap must
    fall through to a re-download (cache_miss phase).

    Inject a stale tmp file by writing junk to the exact path
    `download-client` uses. Since we can't predict the resolved version id
    ahead of time for `latest`, pin the version explicitly.
    """
    info = _per_version_manifest("1.7.10")
    expected_sha1 = info["downloads"]["client"]["sha1"]
    stale_tmp = Path(tempfile.gettempdir()) / "mcmap-client-1.7.10.jar.part"
    # Write garbage that's nowhere near the right sha1.
    stale_tmp.write_bytes(b"not a jar")
    try:
        target = tmp_path / "client.jar"
        events, rc = run_mcmap_json([
            "download-client", "1.7.10", str(target),
        ])
        assert rc == 0, f"download-client failed: {events!r}"
        result = assert_result(events)
        assert result["sha1"] == expected_sha1
        cache_miss = find_event(events, type="progress", phase="cache_miss")
        assert cache_miss is not None, (
            f"stale tmp did not trigger cache_miss path: {events!r}"
        )
    finally:
        # Clean up the tmp the run produced (it's reusable across other
        # cache-hit tests, but it's polite to not leak across CI runs).
        if stale_tmp.exists():
            stale_tmp.unlink()
