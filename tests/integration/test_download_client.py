"""Test the `download-client` subcommand.

Doesn't need a server. Asserts mcmap reports the same sha1 as the manifest.

When MCMAP_FLAVOR is set (CI matrix), this file runs only on the
`vanilla-latest` runner — there's no per-flavor split here, so running it
once is enough.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path

import pytest

from asserts import assert_result, run_mcmap_json
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
