import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace

from PIL import Image, PngImagePlugin
import pytest


@pytest.fixture
def harness(tmp_path, monkeypatch):
    script = Path(__file__).resolve().parents[2] / "tools" / "benchmark_corpus.py"
    spec = importlib.util.spec_from_file_location("benchmark_corpus_under_test", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    corpus = tmp_path / "corpus"
    (corpus / "manifests").mkdir(parents=True)
    (corpus / "world" / "region").mkdir(parents=True)
    palette = corpus / "palette.json"
    palette.write_text("{}")
    binaries = {}
    for name in ["baseline", "candidate", "timer"]:
        binaries[name] = tmp_path / name
        binaries[name].write_text(name)
    state = SimpleNamespace(module=module, root=tmp_path, corpus=corpus, binaries=binaries,
                            calls=[], behaviors={}, clock=0.0, active=False)
    manifest = {"status": "complete", "archive_id": "archive", "regions": [],
                "palette": "palette.json", "worlds": [{
                    "palette": "palette.json", "palette_sha256": module.digest(palette),
                    "minecraft_version": "test-version",
                }]}

    def save_manifest():
        (corpus / "manifests" / "archive.json").write_text(json.dumps(manifest))

    def add_region(name="r.0.0.mca"):
        path = corpus / "world" / "region" / name
        path.write_bytes(name.encode())
        manifest["regions"].append({
            "relative_path": path.relative_to(corpus).as_posix(), "dimension": "overworld",
            "bytes": path.stat().st_size, "chunk_count": 1, "sha256": module.digest(path),
        })
        save_manifest()
        return path

    state.source = add_region()
    state.add_region = add_region
    state.manifest = manifest
    state.save_manifest = save_manifest

    def fake_run(command, **kwargs):
        assert not state.active, "Only one renderer may be active"
        state.active = True
        try:
            usage = None
            if command[0] == str(binaries["timer"]):
                usage = Path(command[4])
                command = command[5:]
            assert Path(command[0]).is_absolute()
            name = Path(command[0]).name
            assert name in {"candidate", "baseline"}
            assert command[command.index("-j") + 1] == "1"
            source = Path(command[command.index("-r") + 1])
            output = Path(command[command.index("-o") + 1])
            image = output / source.with_suffix(".png").name
            assert not image.exists(), "Each repeat must start without its old PNG"
            output.mkdir(parents=True, exist_ok=True)
            (output / "unrelated.txt").write_text("keep")
            count = sum(call["name"] == name for call in state.calls) + 1
            state.calls.append({"name": name, "source": source, "image": image,
                                "command": command, "usage": usage})
            state.clock += 0.1 if name == "candidate" else 0.2
            if usage:
                usage.write_text("0.01 0.02 1000\n" if name == "candidate" else "0.04 0.01 2000\n")
            behavior = state.behaviors.get(name, "ok")
            if callable(behavior):
                behavior = behavior(count)
            x, z = module.region_coordinates(source)
            errors = [
                {"type": "region", "status": "error", "x": x, "z": z,
                 "error": "NBT decode failed" if behavior != "other_error" else "different failure",
                 "elapsed_ms": count},
                {"type": "error", "message": "No regions could be rendered", "elapsed_ms": count},
            ]
            if behavior in {"error", "other_error", "panic_with_error", "different_exit", "crash"}:
                code = {"panic_with_error": 101, "different_exit": 2, "crash": -11}.get(behavior, 1)
                events = [] if behavior == "crash" else errors
                return subprocess.CompletedProcess(command, code, "\n".join(map(json.dumps, events)), "failure")
            if behavior == "empty_events":
                return subprocess.CompletedProcess(command, 0, "", "")
            if behavior != "no_image":
                info = PngImagePlugin.PngInfo()
                if name == "candidate":
                    info.add_text("different compression metadata", "RGBA stays identical")
                Image.new("RGBA", (2, 2), (12, 34, 56, 255)).save(image, pnginfo=info)
                os.utime(image, ns=(source.stat().st_atime_ns, source.stat().st_mtime_ns))
            events = [
                {"type": "region", "status": "rendered", "x": x, "z": z, "output": str(image)},
                {"type": "result", "regions_saved": 1, "elapsed_ms": count},
            ]
            if behavior == "duplicate_region":
                events.insert(0, events[0].copy())
            return subprocess.CompletedProcess(command, 0, "\n".join(map(json.dumps, events)), "")
        finally:
            state.active = False

    def version(command, **kwargs):
        assert Path(command[0]).is_absolute()
        return "mcmap test\n"

    monkeypatch.setattr(module, "subprocess", SimpleNamespace(run=fake_run, check_output=version))
    monkeypatch.setattr(module, "time", SimpleNamespace(
        perf_counter=lambda: state.clock, sleep=module.time.sleep,
    ))

    def invoke(name, output, *extra, expected=0):
        monkeypatch.setattr(sys, "argv", [str(script), "--corpus", str(corpus),
                                         "--binary", str(binaries[name]), "--output", str(output),
                                         *map(str, extra)])
        with pytest.raises(SystemExit) as stopped:
            module.main()
        assert stopped.value.code == expected
        return module.read_rows(output / "results.jsonl")

    state.invoke = invoke
    return state


def test_interleaved_runs_compare_rgba_and_collect_per_process_usage(harness):
    h = harness
    north = h.add_region("r.0.-1.mca")
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline)
    h.calls.clear()
    output = h.root / "paired-run"
    rows = h.invoke("candidate", output, "--reference", baseline, "--reference-binary",
                    h.binaries["baseline"], "--time-binary", h.binaries["timer"], "--repeats", 3)
    assert len(h.calls) == 12
    for offset in range(0, len(h.calls), 2):
        pair = h.calls[offset:offset + 2]
        assert {call["name"] for call in pair} == {"baseline", "candidate"}
        assert pair[0]["source"] == pair[1]["source"]
    row = rows["world/region/r.0.0.mca"]
    assert row["north"] == {"path": "world/region/r.0.-1.mca", "present": True,
                            "sha256": h.module.digest(north)}
    assert row["minecraft_version"] == "test-version"
    assert row["median_cpu_ms"] == 30
    assert row["median_peak_rss_kib"] == 1000
    assert row["reference_medians"]["cpu_ms"] == 50
    assert row["reference_medians"]["peak_rss_kib"] == 2000
    assert row["speedup"] == pytest.approx(2)
    reference = h.module.read_rows(baseline / "results.jsonl")[row["case"]]
    assert row["pixel_sha256"] == reference["pixel_sha256"]
    assert row["png_bytes"] != reference["png_bytes"]
    h.calls.clear()
    h.invoke("candidate", output, "--reference", baseline, "--reference-binary",
             h.binaries["baseline"], "--time-binary", h.binaries["timer"], "--repeats", 3, "--resume")
    assert not h.calls


def test_resume_cannot_add_reference_after_unchecked_run(harness):
    h = harness
    baseline, output = h.root / "baseline-run", h.root / "candidate-run"
    h.invoke("baseline", baseline)
    h.invoke("candidate", output)
    h.calls.clear()
    h.invoke("candidate", output, "--resume", "--reference", baseline, expected=2)
    assert not h.calls


def test_resume_binds_each_reference_result(harness):
    h = harness
    baseline, output = h.root / "baseline-run", h.root / "candidate-run"
    reference = h.invoke("baseline", baseline)
    h.invoke("candidate", output, "--reference", baseline)
    row = next(iter(reference.values()))
    row["pixel_sha256"] = "f" * 64
    (baseline / "results.jsonl").write_text(json.dumps(row) + "\n")
    with pytest.raises(RuntimeError, match="Resumed reference result changed"):
        h.invoke("candidate", output, "--resume", "--reference", baseline)


@pytest.mark.parametrize("mutation", ["add", "change", "remove"])
def test_resume_binds_unlisted_actual_north_context(harness, mutation):
    h = harness
    north = h.source.with_name("r.0.-1.mca")
    if mutation != "add":
        north.write_bytes(b"initial north")
    output = h.root / "candidate-run"
    h.invoke("candidate", output)
    if mutation == "remove":
        north.unlink()
    else:
        north.write_bytes(b"different north")
    with pytest.raises(RuntimeError, match="Resumed inputs changed"):
        h.invoke("candidate", output, "--resume")


def test_reference_binds_north_context_before_rendering(harness):
    h = harness
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline)
    h.source.with_name("r.0.-1.mca").write_bytes(b"new north")
    h.calls.clear()
    with pytest.raises(RuntimeError, match="Baseline inputs differ"):
        h.invoke("candidate", h.root / "candidate-run", "--reference", baseline)
    assert not h.calls


@pytest.mark.parametrize("behavior", ["other_error", "crash", "panic_with_error", "different_exit"])
def test_baseline_error_does_not_accept_different_failure_or_crash(harness, behavior):
    h = harness
    h.behaviors["baseline"] = "error"
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline, expected=1)
    h.behaviors["candidate"] = behavior
    rows = h.invoke("candidate", h.root / "candidate-run", "--reference", baseline, expected=1)
    assert next(iter(rows.values()))["status"] == "error"


def test_same_structured_error_ignores_volatile_event_timing(harness):
    h = harness
    h.behaviors.update(baseline="error", candidate="error")
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline, expected=1)
    rows = h.invoke("candidate", h.root / "candidate-run", "--reference", baseline,
                    "--reference-binary", h.binaries["baseline"])
    assert next(iter(rows.values()))["status"] == "expected_error"


def test_expected_error_still_requires_matching_input_hashes(harness):
    h = harness
    h.behaviors.update(baseline="error", candidate="error")
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline, expected=1)
    h.source.write_bytes(b"different corrupt region")
    h.manifest["regions"][0]["sha256"] = h.module.digest(h.source)
    h.save_manifest()
    h.calls.clear()
    with pytest.raises(RuntimeError, match="Baseline inputs differ"):
        h.invoke("candidate", h.root / "candidate-run", "--reference", baseline)
    assert not h.calls


@pytest.mark.parametrize("name", ["candidate", "baseline"])
@pytest.mark.parametrize("behavior", ["no_image", "empty_events", "duplicate_region"])
def test_each_repeat_requires_new_png_and_success_events(harness, name, behavior):
    h = harness
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline)
    h.calls.clear()
    h.behaviors[name] = lambda count: "ok" if count == 1 else behavior
    with pytest.raises(RuntimeError, match="new PNG|successful region result"):
        h.invoke("candidate", h.root / "candidate-run", "--reference", baseline,
                 "--reference-binary", h.binaries["baseline"], "--repeats", 2)
    assert all((call["image"].parent / "unrelated.txt").is_file() for call in h.calls)


def test_empty_selection_fails(harness):
    harness.invoke("candidate", harness.root / "empty", "--archive", "absent", expected=2)
    assert not harness.calls


def test_unbound_legacy_results_require_rebuild(harness):
    h = harness
    output = h.root / "candidate-run"
    rows = h.invoke("candidate", output)
    row = next(iter(rows.values()))
    del row["north"]
    (output / "results.jsonl").write_text(json.dumps(row) + "\n")
    with pytest.raises(RuntimeError, match="unbound legacy results; rebuild"):
        h.invoke("candidate", output, "--resume")


def test_unbound_reference_metadata_requires_rebuild(harness):
    h = harness
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline)
    metadata = json.loads((baseline / "run.json").read_text())
    del metadata["schema_version"]
    (baseline / "run.json").write_text(json.dumps(metadata))
    h.invoke("candidate", h.root / "candidate-run", "--reference", baseline, expected=2)


def test_paired_reference_cannot_turn_expected_failure_into_crash(harness):
    h = harness
    h.behaviors.update(baseline="error", candidate="error")
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline, expected=1)
    h.behaviors["baseline"] = "crash"
    with pytest.raises(RuntimeError, match="Frozen baseline failure changed"):
        h.invoke("candidate", h.root / "candidate-run", "--reference", baseline,
                 "--reference-binary", h.binaries["baseline"])


def test_reference_binary_identity_and_basename_paths(harness, monkeypatch):
    h = harness
    baseline = h.root / "baseline-run"
    h.invoke("baseline", baseline)
    monkeypatch.chdir(h.root)
    h.binaries["candidate"] = Path("candidate")
    h.invoke("candidate", h.root / "wrong-reference", "--reference", Path("baseline-run"),
             "--reference-binary", Path("candidate"), expected=2)
    h.binaries["candidate"] = h.root / "candidate"
    h.invoke("candidate", h.root / "relative-run", "--reference", Path("baseline-run"),
             "--reference-binary", Path("baseline"), "--time-binary", Path("timer"))
    metadata = json.loads((h.root / "relative-run" / "run.json").read_text())
    assert metadata["time_binary"] == str(h.root / "timer")
    assert all(Path(call["command"][0]).is_absolute() for call in h.calls)
