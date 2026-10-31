# /// script
# requires-python = ">=3.10"
# dependencies = ["Pillow>=10"]
# ///
"""Compare Linux release binaries on equivalent region data without editing inputs."""

import argparse
import hashlib
import json
from pathlib import Path
import random
import re
import statistics
import subprocess
import sys
import time

from PIL import Image


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", nargs=3, action="append", required=True,
                        metavar=("LABEL", "BINARY", "REGIONS"))
    parser.add_argument("--palette", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--time-binary", type=Path, default=Path("/usr/bin/time"))
    parser.add_argument("--threads", nargs="+", type=int, default=[1, 2])
    parser.add_argument("--repeats", type=int, default=15)
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("Run on Linux or WSL to collect per-process CPU and peak RSS")
    if not args.time_binary.is_file():
        parser.error("GNU time is required; install it or set --time-binary")
    if args.repeats < 3 or any(n < 1 for n in args.threads):
        parser.error("Use at least three repeats and positive thread counts")
    cases = {}
    inputs = {args.palette.resolve()}
    for label, binary, regions in args.case:
        if not re.fullmatch(r"[A-Za-z0-9_-]+", label) or label in cases:
            parser.error("Case labels must be unique alphanumeric names with - or _")
        binary, regions = Path(binary).resolve(), Path(regions).resolve()
        files = [regions] if regions.is_file() else sorted(regions.glob("*.mca"))
        if not files:
            parser.error(f"No region files for {label}")
        inputs.update(files)
        cases[label] = (binary, regions)
    hashes = {str(path): digest(path) for path in inputs}
    args.output.mkdir(parents=True, exist_ok=True)
    versions = {
        label: subprocess.check_output([str(binary), "--version"], text=True).strip()
        for label, (binary, _) in cases.items()
    }

    def run(label, threads):
        binary, regions = cases[label]
        directory = args.output / label
        directory.mkdir(exist_ok=True)
        command = [str(binary), "--json", "render", "--split", "-r", str(regions),
                   "-p", str(args.palette.resolve()), "-o", str(directory),
                   "-j", str(threads)]
        usage_file = directory / "usage.txt"
        command = [str(args.time_binary.resolve()), "-f", "%U %S %M", "-o", str(usage_file), *command]
        with (directory / "events.jsonl").open("w") as stdout, \
                (directory / "stderr.log").open("w") as stderr:
            started = time.perf_counter()
            process = subprocess.run(command, stdout=stdout, stderr=stderr)
            elapsed = (time.perf_counter() - started) * 1000
            if process.returncode:
                raise RuntimeError(f"{label} failed; see {directory}")
        user_seconds, system_seconds, peak_rss_kib = map(float, usage_file.read_text().split())
        events = [json.loads(line) for line in (directory / "events.jsonl").read_text().splitlines()]
        regions_rendered = [event for event in events if event.get("type") == "region"]
        if not regions_rendered or any(event["status"] != "rendered" for event in regions_rendered):
            raise RuntimeError(f"{label} did not render every region: {regions_rendered}")
        pixels = {}
        for event in regions_rendered:
            with Image.open(event["output"]) as image:
                rgba = image.convert("RGBA")
                pixels[f'{event["x"]},{event["z"]}'] = {
                    "size": rgba.size,
                    "sha256": hashlib.sha256(rgba.tobytes()).hexdigest(),
                    "visible_pixels": sum(value != 0 for value in rgba.getchannel("A").tobytes()),
                }
        return {
            "case": label, "threads": threads, "wall_ms": elapsed,
            "render_ms": events[-1]["elapsed_ms"],
            "cpu_ms": (user_seconds + system_seconds) * 1000,
            "peak_rss_kib": int(peak_rss_kib),
            "region_order": [f'{event["x"]},{event["z"]}' for event in regions_rendered],
        }, pixels

    expected = None
    rows = []
    rng = random.Random(263)
    for threads in args.threads:
        for label in cases:
            _, pixels = run(label, threads)
            if expected is None:
                expected = pixels
            if pixels != expected:
                raise AssertionError(f"Pixel mismatch in {label}")
        for repeat in range(args.repeats):
            order = list(cases)
            rng.shuffle(order)
            for label in order:
                measurement, pixels = run(label, threads)
                if pixels != expected:
                    raise AssertionError(f"Pixel mismatch in {label}, repeat {repeat}")
                rows.append({"repeat": repeat, **measurement})
        print(f"Completed {threads} thread(s)", flush=True)
    if hashes != {str(path): digest(path) for path in inputs}:
        raise AssertionError("Input files changed during benchmark")
    summary = []
    for threads in args.threads:
        for label in cases:
            group = [row for row in rows if row["threads"] == threads and row["case"] == label]
            summary.append({
                "case": label, "threads": threads, "runs": len(group),
                **{f"median_{key}": statistics.median(row[key] for row in group)
                   for key in ["wall_ms", "render_ms", "cpu_ms", "peak_rss_kib"]},
                "min_wall_ms": min(row["wall_ms"] for row in group),
                "max_wall_ms": max(row["wall_ms"] for row in group),
            })
    report = {"versions": versions, "cases": args.case, "input_sha256": hashes,
              "pixels": expected, "summary": summary, "measurements": rows}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
