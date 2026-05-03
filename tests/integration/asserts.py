"""Helpers for asserting on mcmap's NDJSON event stream and PNG outputs."""

from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
from pathlib import Path

log = logging.getLogger(__name__)


def mcmap_bin() -> Path:
    """Locate the mcmap binary.

    Priority: $MCMAP_BIN env var, then target/release/mcmap relative to the
    repo root (3 levels up from this file).
    """
    env = os.environ.get("MCMAP_BIN")
    if env:
        p = Path(env)
        if not p.exists():
            raise RuntimeError(f"$MCMAP_BIN={env} does not exist")
        return p
    # tests/integration/asserts.py -> repo root is 2 dirs up.
    repo_root = Path(__file__).resolve().parents[2]
    candidate = repo_root / "target" / "release" / "mcmap"
    if candidate.exists():
        return candidate
    candidate_exe = candidate.with_suffix(".exe")
    if candidate_exe.exists():
        return candidate_exe
    raise RuntimeError(
        "mcmap binary not found. Set $MCMAP_BIN or run `cargo build --release` first."
    )


def run_mcmap_json(args: list[str]) -> tuple[list[dict], int]:
    """Run mcmap --json with `args`, return (parsed events, exit code).

    The events list is parsed line-by-line from stdout. Any non-JSON line is
    skipped with a warning. The terminal event (last in the list) should be
    `type=result` on success or `type=error` on failure.
    """
    cmd = [str(mcmap_bin()), "--json", *args]
    log.info("RUN %s", " ".join(cmd))
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    events: list[dict] = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            log.warning("non-JSON stdout line: %r", line)
    if proc.stderr:
        log.debug("mcmap stderr:\n%s", proc.stderr)
    return events, proc.returncode


def assert_result(events: list[dict]) -> dict:
    """Assert the event stream ended with a `result` event; return it."""
    assert events, "no events emitted"
    last = events[-1]
    assert last.get("type") == "result", f"final event was not result: {last!r}"
    return last


def assert_error(events: list[dict]) -> dict:
    """Assert the event stream ended with an `error` event; return it.

    mcmap commands always emit a final type=error JSON event under --json
    when execute() returns Err. (clap-level argument errors exit before
    JSON output engages — those are stderr-only and out of scope here.)
    """
    assert events, "no events emitted"
    last = events[-1]
    assert last.get("type") == "error", f"final event was not error: {last!r}"
    assert isinstance(last.get("message"), str) and last["message"], (
        f"error event missing message: {last!r}"
    )
    return last


def find_event(events: list[dict], **fields) -> dict | None:
    """Return the first event matching all `fields`, or None."""
    for ev in events:
        if all(ev.get(k) == v for k, v in fields.items()):
            return ev
    return None


def png_pixel(path: Path, x: int, y: int) -> tuple[int, int, int, int]:
    """Return the RGBA pixel at (x, y) in `path`. Pads short tuples with alpha=255."""
    from PIL import Image
    with Image.open(path) as im:
        im = im.convert("RGBA")
        return im.getpixel((x, y))


def colors_close(a: tuple[int, ...], b: tuple[int, ...], *, tol: int = 4) -> bool:
    """RGB(A) tolerance check. Defaults to a tight tolerance.

    Render's color comes straight out of the palette JSON; mcmap doesn't
    quantize, so the rendered pixel should match the palette tuple exactly.
    Tolerance protects against alpha pre-multiply rounding on edge biome
    blocks; for fixtures placed by /setblock we expect tol=0 in practice.
    """
    return all(abs(int(x) - int(y)) <= tol for x, y in zip(a, b))


def copy_tree(src: Path, dst: Path) -> Path:
    """Copy a directory tree, replacing dst if it exists."""
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    return dst


def overworld_region_dir(work_dir: Path) -> Path:
    """Return the overworld region directory inside `work_dir`.

    Recent Minecraft versions (observed on 26.1.x; the layout shift goes back
    several versions before that) move every dimension's chunks under
    `world/dimensions/<ns>/<id>/{region,entities,poi}/`. Older versions keep
    the overworld's chunks at `world/region/` directly. Probe both.
    """
    modern = work_dir / "world" / "dimensions" / "minecraft" / "overworld" / "region"
    if modern.is_dir():
        return modern
    return work_dir / "world" / "region"
