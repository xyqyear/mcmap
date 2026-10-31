# /// script
# requires-python = ">=3.11"
# dependencies = ["Pillow>=10"]
# ///
"""Render an archived-world corpus sequentially and compare exact RGBA output."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import statistics
import subprocess
import time

from PIL import Image, ImageChops

SCHEMA_VERSION = 2


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_rows(path):
    if not path.exists():
        return {}
    return {row["case"]: row for line in path.read_text(encoding="utf-8").splitlines()
            if line and (row := json.loads(line))}


def validate_rows(rows, description):
    required = {"schema_version", "source_sha256", "palette_sha256", "north",
                "reference", "returncode", "error_signature", "status"}
    for case, row in rows.items():
        if row.get("schema_version") != SCHEMA_VERSION or not required <= row.keys():
            raise RuntimeError(f"{description} contains unbound legacy results; rebuild it: {case}")


def reference_binding(row):
    return {field: row.get(field) for field in [
        "source_sha256", "palette_sha256", "north", "status", "returncode",
        "error_signature", "pixel_sha256", "size", "png_bytes",
    ]}


def file_state(corpus, relative_path):
    path = corpus / relative_path
    present = path.exists()
    if present and not path.is_file():
        raise RuntimeError(f"Expected a corpus file: {relative_path}")
    return {"path": relative_path, "present": present,
            "sha256": digest(path) if present else None}


def region_coordinates(source):
    match = re.fullmatch(r"r\.(-?\d+)\.(-?\d+)\.mca", source.name)
    if not match:
        raise RuntimeError(f"Invalid region filename: {source}")
    return tuple(map(int, match.groups()))


def north_context(corpus, source):
    x, z = region_coordinates(source)
    path = source.with_name(f"r.{x}.{z - 1}.mca")
    return file_state(corpus, path.relative_to(corpus).as_posix())


def error_signature(events):
    signature = []
    for event in events:
        if event.get("type") == "region" and event.get("status") == "error":
            signature.append({field: event.get(field) for field in ["type", "x", "z", "error"]})
        elif event.get("type") == "error":
            signature.append({field: event.get(field) for field in ["type", "message"]})
    return signature


def same_failure(outcome, previous):
    return (previous["status"] in {"error", "expected_error"}
            and outcome["returncode"] in {0, 1, 2}
            and outcome["returncode"] == previous["returncode"]
            and bool(outcome["error_signature"])
            and outcome["error_signature"] == previous["error_signature"])


def timing_summary(rows):
    times = sorted(row["median_wall_ms"] for row in rows)
    return {
        "cases": len(times),
        "median_ms": statistics.median(times) if times else None,
        "p95_ms": times[math.ceil(len(times) * 0.95) - 1] if times else None,
        "p99_ms": times[math.ceil(len(times) * 0.99) - 1] if times else None,
        "max_ms": max(times) if times else None,
        "under_50_ms": sum(value < 50 for value in times),
    }


def measure(command, time_binary, usage_file, image_path):
    image_path.unlink(missing_ok=True)
    if time_binary:
        usage_file.unlink(missing_ok=True)
        command = [str(time_binary), "-f", "%U %S %M", "-o", str(usage_file), *command]
    started = time.perf_counter()
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
    measurement = {"wall_ms": (time.perf_counter() - started) * 1000}
    events = [json.loads(line) for line in result.stdout.splitlines() if line]
    if time_binary:
        user, system, rss = usage_file.read_text().splitlines()[-1].split()
        measurement.update(cpu_ms=(float(user) + float(system)) * 1000, peak_rss_kib=int(rss))
    if events and "elapsed_ms" in events[-1]:
        measurement["render_ms"] = events[-1]["elapsed_ms"]
    return result, events, measurement


def inspect_run(result, events, image_path, source, mtime_tolerance_ms):
    signature = error_signature(events)
    outcome = {"returncode": result.returncode, "error_signature": signature}
    if result.returncode or signature:
        return {**outcome, "status": "error"}
    regions = [event for event in events if event.get("type") == "region"]
    results = [event for event in events if event.get("type") == "result"]
    x, z = region_coordinates(source)
    if (len(regions) != 1 or len(results) != 1 or events[-1] != results[0]
            or regions[0].get("status") != "rendered"
            or (regions[0].get("x"), regions[0].get("z")) != (x, z)
            or results[0].get("regions_saved") != 1):
        raise RuntimeError(f"Missing successful region result: {source}")
    if not image_path.is_file():
        raise RuntimeError(f"Successful result did not create a new PNG: {source}")
    if Path(regions[0].get("output") or "").resolve() != image_path.resolve():
        raise RuntimeError(f"Region result names a different PNG: {source}")
    with Image.open(image_path) as image:
        rgba = image.convert("RGBA")
        outcome.update(
            pixel_sha256=hashlib.sha256(rgba.tobytes()).hexdigest(),
            visible_pixels=sum(value != 0 for value in rgba.getchannel("A").tobytes()),
            size=list(rgba.size), png_bytes=image_path.stat().st_size,
        )
    outcome["mtime_delta_ns"] = image_path.stat().st_mtime_ns - source.stat().st_mtime_ns
    if abs(outcome["mtime_delta_ns"]) > mtime_tolerance_ms * 1_000_000:
        raise RuntimeError(f"Output mtime differs from source: {source}")
    outcome["status"] = "ok" if outcome["visible_pixels"] else "transparent"
    return outcome


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--reference-binary", type=Path,
                        help="Remeasure the frozen reference in interleaved order with each candidate")
    parser.add_argument("--time-binary", type=Path, help="GNU time executable for per-process CPU and peak RSS")
    parser.add_argument("--reference-only", action="store_true")
    parser.add_argument("--archive", action="append")
    parser.add_argument("--repeats", type=int, default=1)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--calculate-heights", action="store_true")
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--pause-file", type=Path,
                        help="Pause between cases while this coordination marker exists")
    parser.add_argument("--mtime-tolerance-ms", type=float, default=1)
    parser.add_argument("--source-sha", default="uncommitted")
    parser.add_argument("--conditions", default="Unqualified timing; record machine load separately")
    args = parser.parse_args()
    if args.reference_only and not args.reference:
        parser.error("--reference-only requires --reference")
    if args.reference_binary and not args.reference:
        parser.error("--reference-binary requires --reference")
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    if args.limit is not None and args.limit < 1:
        parser.error("--limit must be positive")
    for name in ["corpus", "binary", "output", "reference", "reference_binary", "time_binary"]:
        path = getattr(args, name)
        if path is not None:
            setattr(args, name, path.resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    rows_file = args.output / "results.jsonl"
    completed = read_rows(rows_file) if args.resume else {}
    if rows_file.exists() and not args.resume:
        parser.error("Output already contains results; use a fresh directory or --resume")
    reference = read_rows(args.reference / "results.jsonl") if args.reference else None
    validate_rows(completed, "Resumed run")
    if reference is not None:
        validate_rows(reference, "Reference")
    binary_hash = digest(args.binary)
    metadata_file = args.output / "run.json"
    metadata = {
        "schema_version": SCHEMA_VERSION, "corpus": str(args.corpus),
        "binary_sha256": binary_hash, "source_sha": args.source_sha,
        "version": subprocess.check_output([str(args.binary), "--version"], text=True).strip(),
        "threads": 1, "sequential_processes": True, "repeats": args.repeats,
        "calculate_heights": args.calculate_heights, "conditions": args.conditions,
        "reference_binary_sha256": digest(args.reference_binary) if args.reference_binary else None,
        "time_binary": str(args.time_binary) if args.time_binary else None,
        "time_binary_sha256": digest(args.time_binary) if args.time_binary else None,
        "reference": None, "mtime_tolerance_ms": args.mtime_tolerance_ms,
        "platform": platform.platform(), "machine": platform.machine(),
        "logical_cpus": os.cpu_count(),
    }
    if args.reference:
        reference_metadata = json.loads((args.reference / "run.json").read_text())
        if reference_metadata.get("schema_version") != SCHEMA_VERSION:
            parser.error("Reference metadata is unbound; rebuild the reference")
        metadata["reference"] = {
            "mode": "paired" if args.reference_binary else "compare",
            "path": str(args.reference),
            "binary_sha256": reference_metadata["binary_sha256"],
            "calculate_heights": reference_metadata["calculate_heights"],
        }
        if (args.reference_binary
                and reference_metadata["binary_sha256"] != metadata["reference_binary_sha256"]):
            parser.error("Reference binary does not match the frozen reference outputs")
    if args.resume and metadata_file.exists():
        previous_metadata = json.loads(metadata_file.read_text())
        if previous_metadata.get("schema_version") != SCHEMA_VERSION:
            parser.error("Resumed metadata is unbound; rebuild the run")
        for field in ["corpus", "binary_sha256", "source_sha", "repeats", "calculate_heights",
                      "conditions", "reference", "reference_binary_sha256", "time_binary",
                      "time_binary_sha256", "mtime_tolerance_ms", "platform", "machine", "logical_cpus"]:
            if previous_metadata.get(field) != metadata[field]:
                parser.error(f"Cannot change {field} in a resumed run")
    elif completed:
        parser.error("Resumed results have no metadata; rebuild the run")
    metadata_file.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    cases = []
    contexts = {}
    for manifest_path in sorted((args.corpus / "manifests").glob("*.json")):
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if not isinstance(manifest, dict):
            continue
        archive = manifest.get("archive_id")
        if manifest.get("status") != "complete" or not archive:
            continue
        if args.archive and archive not in args.archive:
            continue
        for context in manifest.get("context_regions", []):
            contexts[context["relative_path"]] = context["sha256"]
        for region in manifest["regions"]:
            if args.reference_only and region["relative_path"] not in reference:
                continue
            cases.append((archive, manifest, region))
    cases.sort(key=lambda item: (item[0], item[2]["relative_path"]))
    if args.limit is not None:
        cases = cases[:args.limit]
    if not cases:
        parser.error("No corpus cases selected")
    palettes = {}
    for name, expected in contexts.items():
        if digest(args.corpus / name) != expected:
            raise RuntimeError(f"North context digest mismatch: {name}")
    rng = random.Random(263)
    failures = 0
    observed_north = {}
    with rows_file.open("a", encoding="utf-8") as report:
        for number, (archive, manifest, region) in enumerate(cases, 1):
            if args.pause_file and args.pause_file.exists():
                print(f"Paused before case {number}: {args.pause_file}", flush=True)
                while args.pause_file.exists():
                    time.sleep(0.1)
            case = region["relative_path"]
            source = args.corpus / case
            palette_name = region.get("palette") or manifest["palette"]
            palette = args.corpus / palette_name
            palette_metadata = next(
                (world for world in manifest.get("worlds", []) if world["palette"] == palette_name),
                manifest,
            )
            if palette_name not in palettes:
                palettes[palette_name] = digest(palette)
            if palette_name == palette_metadata.get("palette") and palette_metadata.get("palette_sha256"):
                if palettes[palette_name] != palette_metadata["palette_sha256"]:
                    raise RuntimeError(f"Corpus palette digest mismatch: {palette_name}")
            source_hash = digest(source)
            if source_hash != region["sha256"]:
                raise RuntimeError(f"Corpus source digest mismatch: {case}")
            north = north_context(args.corpus, source)
            if observed_north.setdefault(north["path"], north) != north:
                raise RuntimeError(f"North context changed: {case}")
            baseline = reference.get(case) if reference is not None else None
            if reference is not None:
                if baseline is None:
                    raise RuntimeError(f"Missing baseline case: {case}")
                if (baseline["source_sha256"] != source_hash
                        or baseline["palette_sha256"] != palettes[palette_name]
                        or baseline["north"] != north):
                    raise RuntimeError(f"Baseline inputs differ: {case}")
                if baseline["status"] not in {"ok", "transparent", "error", "expected_error"}:
                    raise RuntimeError(f"Baseline case is not verified: {case}")
            binding = reference_binding(baseline) if baseline is not None else None
            if case in completed:
                prior = completed[case]
                if (prior["source_sha256"] != source_hash
                        or prior["palette_sha256"] != palettes[palette_name]
                        or prior["north"] != north):
                    raise RuntimeError(f"Resumed inputs changed: {case}")
                if prior["reference"] != binding:
                    raise RuntimeError(f"Resumed reference result changed: {case}")
                continue
            case_id = hashlib.sha256(case.encode()).hexdigest()[:20]
            directory = args.output / "images" / case_id
            directory.mkdir(parents=True, exist_ok=True)
            image_path = directory / source.with_suffix(".png").name
            command = [str(args.binary), "--json", "render", "-r", str(source),
                       "-p", str(palette), "-o", str(directory), "--split",
                       "--preserve-mtime", "-j", "1"]
            if args.calculate_heights:
                command.append("--calculate-heights")
            row = {
                "schema_version": SCHEMA_VERSION, "north": north, "reference": binding,
                "case": case, "archive": archive, "dimension": region["dimension"],
                "minecraft_version": region.get("minecraft_version") or palette_metadata.get("minecraft_version"),
                "bytes": region["bytes"], "chunk_count": region["chunk_count"],
                "source_sha256": source_hash, "palette": palette_name,
                "palette_sha256": palettes[palette_name], "wall_ms": [], "render_ms": [],
                "cpu_ms": [], "peak_rss_kib": [], "reference_measurements": [],
                "image": image_path.relative_to(args.output).as_posix(),
            }
            reference_command = None
            if args.reference_binary:
                reference_directory = args.output / "reference-images" / case_id
                reference_directory.mkdir(parents=True, exist_ok=True)
                reference_image = reference_directory / image_path.name
                reference_command = command.copy()
                reference_command[0] = str(args.reference_binary)
                reference_command[reference_command.index("-o") + 1] = str(reference_directory)
            for repeat in range(args.repeats):
                order = ["candidate", "reference"] if reference_command else ["candidate"]
                rng.shuffle(order)
                for label in order:
                    if label == "candidate":
                        result, events, measurement = measure(
                            command, args.time_binary, directory / "usage.txt", image_path)
                        outcome = inspect_run(result, events, image_path, source, args.mtime_tolerance_ms)
                    else:
                        before_result, before_events, before_measurement = measure(
                            reference_command, args.time_binary, reference_directory / "usage.txt",
                            reference_image)
                        before = inspect_run(before_result, before_events, reference_image, source,
                                             args.mtime_tolerance_ms)
                        if before["status"] == "error":
                            if not same_failure(before, baseline):
                                raise RuntimeError(f"Frozen baseline failure changed: {case}")
                        else:
                            if (before["pixel_sha256"] != baseline.get("pixel_sha256")
                                    or before["size"] != baseline.get("size")):
                                raise RuntimeError(f"Frozen baseline output changed: {case}")
                            row["reference_measurements"].append(before_measurement)
                if outcome["status"] == "error":
                    expected_failure = baseline is not None and same_failure(outcome, baseline)
                    row.update(outcome, status="expected_error" if expected_failure else "error",
                               events=events, stderr=result.stderr[-4000:])
                    failures += not expected_failure
                    break
                if repeat and (outcome["pixel_sha256"] != row["pixel_sha256"]
                               or outcome["size"] != row["size"]):
                    raise RuntimeError(f"Nondeterministic pixels: {case}")
                row.update(outcome)
                if baseline is not None and (
                        baseline.get("pixel_sha256") != outcome["pixel_sha256"]
                        or baseline.get("size") != outcome["size"]):
                    if baseline.get("pixel_sha256"):
                        with Image.open(args.reference / baseline["image"]) as before, Image.open(image_path) as image:
                            if before.size == image.size:
                                ImageChops.difference(before.convert("RGBA"), image.convert("RGBA")).save(
                                    directory / "difference.png")
                    row["status"] = "mismatch"
                    failures += 1
                for field in ["wall_ms", "render_ms", "cpu_ms", "peak_rss_kib"]:
                    if field in measurement:
                        row[field].append(measurement[field])
            if digest(source) != source_hash:
                raise RuntimeError(f"Source changed while rendering: {case}")
            if north_context(args.corpus, source) != north:
                raise RuntimeError(f"North context changed while rendering: {case}")
            for field in ["wall_ms", "render_ms", "cpu_ms", "peak_rss_kib"]:
                if row[field]:
                    row[f"median_{field}"] = statistics.median(row[field])
            if row["reference_measurements"]:
                row["reference_medians"] = {
                    field: statistics.median(item[field] for item in row["reference_measurements"])
                    for field in row["reference_measurements"][0]
                }
                if row.get("median_wall_ms"):
                    row["speedup"] = row["reference_medians"]["wall_ms"] / row["median_wall_ms"]
            report.write(json.dumps(row, ensure_ascii=False) + "\n")
            report.flush()
            if number % 25 == 0 or number == len(cases):
                print(f"{number}/{len(cases)} {archive} {row['status']}", flush=True)
    for name, expected in palettes.items():
        if digest(args.corpus / name) != expected:
            raise RuntimeError(f"Palette changed: {name}")
    for name, expected in contexts.items():
        if digest(args.corpus / name) != expected:
            raise RuntimeError(f"North context changed: {name}")
    for name, expected in observed_north.items():
        if file_state(args.corpus, name) != expected:
            raise RuntimeError(f"North context changed: {name}")
    if digest(args.binary) != binary_hash:
        raise RuntimeError("Candidate binary changed during the run")
    if (args.reference_binary
            and digest(args.reference_binary) != metadata["reference_binary_sha256"]):
        raise RuntimeError("Reference binary changed during the run")
    selected = {region["relative_path"] for _, _, region in cases}
    rows = [row for case, row in read_rows(rows_file).items() if case in selected]
    good = [row for row in rows if row["status"] == "ok"]
    summary = {
        **timing_summary(good),
        "cases": len(rows), "visible_cases": len(good),
        "archives": len({row["archive"] for row in good}),
        "unique_region_sha256": len({row["source_sha256"] for row in good}),
        "unique_region_palette_pairs": len({(row["source_sha256"], row["palette_sha256"]) for row in good}),
        "visible_region_bytes": sum(row["bytes"] for row in good),
        "errors": sum(row["status"] == "error" for row in rows),
        "expected_errors": sum(row["status"] == "expected_error" for row in rows),
        "mismatches": sum(row["status"] == "mismatch" for row in rows),
        "transparent": sum(row["status"] == "transparent" for row in rows),
        "by_archive": {
            archive: timing_summary([row for row in good if row["archive"] == archive])
            for archive in sorted({row["archive"] for row in good})
        },
        "by_version": {
            version: timing_summary([row for row in good if (row.get("minecraft_version") or "unknown") == version])
            for version in sorted({row.get("minecraft_version") or "unknown" for row in good})
        },
        "by_stored_chunks": {
            label: timing_summary([row for row in good if low <= row["chunk_count"] <= high])
            for label, low, high in [("1-255", 1, 255), ("256-767", 256, 767), ("768-1024", 768, 1024)]
        },
        "by_visible_pixels": {
            label: timing_summary([row for row in good if low <= row["visible_pixels"] <= high])
            for label, low, high in [("1-65535", 1, 65535), ("65536-196607", 65536, 196607),
                                     ("196608-262144", 196608, 262144)]
        },
        "conditions": args.conditions,
    }
    paired = [row for row in good if row.get("reference_medians")]
    if paired:
        summary["comparison"] = {
            "cases": len(paired),
            "baseline": timing_summary([
                {"median_wall_ms": row["reference_medians"]["wall_ms"]} for row in paired
            ]),
            "median_speedup": statistics.median(row["speedup"] for row in paired),
            "total_median_wall_ms": sum(row["median_wall_ms"] for row in paired),
            "baseline_total_median_wall_ms": sum(row["reference_medians"]["wall_ms"] for row in paired),
            "candidate_png_bytes": sum(row["png_bytes"] for row in paired),
            "baseline_png_bytes": sum(reference[row["case"]]["png_bytes"] for row in paired),
        }
        for field in ["cpu_ms", "peak_rss_kib"]:
            measured = [row for row in paired if f"median_{field}" in row]
            if measured:
                summary["comparison"][field] = {
                    "candidate_median": statistics.median(row[f"median_{field}"] for row in measured),
                    "baseline_median": statistics.median(row["reference_medians"][field] for row in measured),
                    "candidate_max": max(row[f"median_{field}"] for row in measured),
                    "baseline_max": max(row["reference_medians"][field] for row in measured),
                }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2), flush=True)
    raise SystemExit(bool(failures or summary["errors"] or summary["mismatches"]))


if __name__ == "__main__":
    main()
