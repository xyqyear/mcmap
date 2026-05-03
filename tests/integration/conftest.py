"""Pytest fixtures for the integration test harness.

The whole suite is gated behind MCMAP_INTEGRATION_TESTS=1. On CI we set this
explicitly via the workflow; locally you'd `MCMAP_INTEGRATION_TESTS=1 uv run
pytest`. The MCMAP_FLAVOR env var picks a single flavor for the run (one
flavor per CI matrix runner); leave it unset to run every flavor that the
test applies to.
"""

from __future__ import annotations

import logging
import os
import secrets
import shutil
from pathlib import Path

import pytest

import flavors
from cache import resolve_latest_release, java_major_for
from flavors import Flavor


def pytest_configure(config: pytest.Config) -> None:
    if not os.environ.get("MCMAP_INTEGRATION_TESTS"):
        pytest.exit(
            "Integration tests are gated. Set MCMAP_INTEGRATION_TESTS=1 to run.",
            returncode=0,
        )

    # Resolve "latest" once per session and patch the flavor's mc_version
    # and java_major in place.
    latest_id = resolve_latest_release()
    java_major = java_major_for(latest_id)
    for f in flavors.all_flavors():
        if f.id == "vanilla-latest":
            f.mc_version = latest_id
            f.java_major = java_major

    logging.basicConfig(
        level=os.environ.get("MCMAP_TEST_LOG_LEVEL", "INFO"),
        format="%(asctime)s %(name)s %(levelname)s %(message)s",
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Honor MCMAP_FLAVOR — when set, deselect parametrized cases for other flavors."""
    only = os.environ.get("MCMAP_FLAVOR")
    if not only:
        return
    keep, skip = [], []
    for item in items:
        flavor = _flavor_id_of(item)
        if flavor is None or flavor == only:
            keep.append(item)
        else:
            skip.append(item)
    if skip:
        config.hook.pytest_deselected(items=skip)
    items[:] = keep


def _flavor_id_of(item: pytest.Item) -> str | None:
    cm = item.get_closest_marker("flavor")
    if cm and cm.args:
        return cm.args[0]
    # Fall back to parametrize id "[flavor=<id>]" pattern.
    for word in item.callspec.params.values() if hasattr(item, "callspec") else []:
        if isinstance(word, Flavor):
            return word.id
    return None


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def run_root(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A persistent per-session run root under /tmp/mcmap-test-run-<rand>.

    Stays alive for the session; wiped on session finish unless KEEP_RUN=1.
    """
    keep = os.environ.get("KEEP_RUN") == "1"
    base = Path("/tmp") / f"mcmap-test-run-{secrets.token_hex(8)}"
    base.mkdir(parents=True, exist_ok=True)
    yield base
    if not keep:
        shutil.rmtree(base, ignore_errors=True)


@pytest.fixture
def work_dir(request: pytest.FixtureRequest, run_root: Path, _flavor: Flavor) -> Path:
    """A per-test working directory under run_root/<flavor>/<test-name>/."""
    safe_name = request.node.name.replace("/", "_").replace("[", "_").replace("]", "_")
    p = run_root / _flavor.id / safe_name
    if p.exists():
        shutil.rmtree(p)
    p.mkdir(parents=True)
    return p


def _all_flavor_ids() -> list[str]:
    return [f.id for f in flavors.all_flavors()]


def pytest_generate_tests(metafunc: pytest.Metafunc) -> None:
    """Provide a `_flavor` parameter to any test that requests it.

    Tests can narrow the flavor list by setting `pytestmark` to
    `pytest.mark.parametrize('_flavor', flavors.legacy_flavors(), ...)`,
    or by listing the ids of the flavors they apply to via the
    `applicable_flavors` attribute.
    """
    if "_flavor" not in metafunc.fixturenames:
        return
    only = os.environ.get("MCMAP_FLAVOR")
    fl = flavors.all_flavors()
    # Test-defined narrowing.
    only_for = getattr(metafunc.function, "applicable_flavors", None)
    if only_for is not None:
        fl = [f for f in fl if f.id in only_for]
    if only:
        fl = [f for f in fl if f.id == only]
    metafunc.parametrize("_flavor", fl, ids=[f.id for f in fl])
