"""Idempotent download/install cache for the integration test harness.

Everything lives under MCMAP_TEST_CACHE (default /tmp/mcmap-test-cache):

    jdk/<major>/<extracted-jdk>/        # Adoptium Temurin or Microsoft OpenJDK
    servers/vanilla/<mc>/server.jar
    servers/forge/<mc>/installer.jar
    servers/forge/<mc>/installed/       # output of `--installServer`
    mods/<key>.jar
    manifests/version_manifest_v2.json
    manifests/<mc>.json                 # per-version manifest
    manifests/latest.txt                # pinned latest release id

Functions are idempotent — they return cached paths if present and intact.
sha1 is checked when the manifest provides one. JDKs are extracted on first
use; subsequent runs reuse the extracted directory.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import shutil
import subprocess
import tarfile
import time
from pathlib import Path

import requests

log = logging.getLogger(__name__)


CACHE_ROOT = Path(os.environ.get("MCMAP_TEST_CACHE", "/tmp/mcmap-test-cache"))
MANIFEST_URL = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"
MANIFEST_TTL_SECONDS = 24 * 3600

# Pinned recommended Forge builds. These are the "recommended" tags off
# files.minecraftforge.net at the time the harness was written.
FORGE_BUILDS = {
    "1.7.10":  "10.13.4.1614-1.7.10",
    "1.12.2":  "14.23.5.2859",
}

# Mod download endpoints. NEID is pinned to 1.4.6 — the last release using
# the original ASM transformer; later releases (1.5+, 2.x) switched to Mixin
# and require GTNH's custom Forge runtime, which stock Forge 1.7.10 lacks.
# 1.4.6 still writes the `Blocks16` NBT tag so mcmap's NEID code path is
# exercised (16-bit metadata via `Data16` is a 2.x-only addition; not
# required to validate the renderer).
# REI is fetched via Modrinth — the GitHub repo has no published releases.
MOD_SOURCES: dict[str, dict] = {
    "notenoughids-1.7.10": {
        "kind": "direct-url",
        "url": (
            "https://github.com/GTNewHorizons/NotEnoughIds/releases/download/"
            "1.4.6/notenoughIDs-1.7.10-1.4.6.jar"
        ),
    },
    "rei-1.12.2": {
        "kind": "modrinth",
        "project": "reid",
        "game_version": "1.12.2",
        "loader": "forge",
    },
    # REI 2.x is Mixin-based and depends on MixinBooter at coremod-load time.
    # Fetch the latest 1.12.2 build alongside the REI jar.
    "mixinbooter-1.12.2": {
        "kind": "modrinth",
        "project": "mixinbooter",
        "game_version": "1.12.2",
        "loader": "forge",
    },
}


# ---------------------------------------------------------------------------
# Path helpers
# ---------------------------------------------------------------------------


def _ensure(p: Path) -> Path:
    p.mkdir(parents=True, exist_ok=True)
    return p


def cache_root() -> Path:
    return _ensure(CACHE_ROOT)


def manifests_dir() -> Path:
    return _ensure(cache_root() / "manifests")


def jdk_dir(major: int) -> Path:
    return _ensure(cache_root() / "jdk" / str(major))


def vanilla_server_dir(mc_version: str) -> Path:
    return _ensure(cache_root() / "servers" / "vanilla" / mc_version)


def forge_dir(mc_version: str) -> Path:
    return _ensure(cache_root() / "servers" / "forge" / mc_version)


def mod_dir() -> Path:
    return _ensure(cache_root() / "mods")


# ---------------------------------------------------------------------------
# HTTP helpers
# ---------------------------------------------------------------------------


def _http_get(url: str, **kwargs) -> requests.Response:
    log.info("GET %s", url)
    r = requests.get(url, timeout=120, **kwargs)
    r.raise_for_status()
    return r


def _sha1(path: Path) -> str:
    h = hashlib.sha1()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _download(url: str, dest: Path, *, expected_sha1: str | None = None) -> Path:
    if dest.exists() and (expected_sha1 is None or _sha1(dest) == expected_sha1):
        return dest
    tmp = dest.with_suffix(dest.suffix + ".part")
    with _http_get(url, stream=True) as r:
        with tmp.open("wb") as f:
            for chunk in r.iter_content(chunk_size=1 << 20):
                f.write(chunk)
    if expected_sha1 is not None:
        got = _sha1(tmp)
        if got != expected_sha1:
            tmp.unlink()
            raise RuntimeError(f"sha1 mismatch for {url}: got {got}, want {expected_sha1}")
    tmp.replace(dest)
    return dest


# ---------------------------------------------------------------------------
# Vanilla manifest + server jar
# ---------------------------------------------------------------------------


def _load_manifest(force: bool = False) -> dict:
    path = manifests_dir() / "version_manifest_v2.json"
    if not force and path.exists():
        age = time.time() - path.stat().st_mtime
        if age < MANIFEST_TTL_SECONDS:
            return json.loads(path.read_text())
    data = _http_get(MANIFEST_URL).json()
    path.write_text(json.dumps(data))
    return data


def resolve_latest_release() -> str:
    """Resolve the latest stable release id, with per-session pinning.

    If manifests/latest.txt exists and the cached manifest is fresh, return
    the pinned id. Otherwise re-resolve from the manifest, write the pin,
    and return.
    """
    pin_path = manifests_dir() / "latest.txt"
    manifest = _load_manifest()
    fresh_id = manifest["latest"]["release"]
    if pin_path.exists():
        pinned = pin_path.read_text().strip()
        # If the manifest just got refreshed and the latest changed, drop
        # the pin so we move forward.
        if pinned == fresh_id:
            return pinned
    pin_path.write_text(fresh_id)
    return fresh_id


def _per_version_manifest(mc_version: str) -> dict:
    cached = manifests_dir() / f"{mc_version}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    manifest = _load_manifest()
    entry = next((v for v in manifest["versions"] if v["id"] == mc_version), None)
    if entry is None:
        # Try a forced refresh (newly-released versions).
        manifest = _load_manifest(force=True)
        entry = next((v for v in manifest["versions"] if v["id"] == mc_version), None)
    if entry is None:
        raise RuntimeError(f"version {mc_version!r} not in Mojang manifest")
    data = _http_get(entry["url"]).json()
    cached.write_text(json.dumps(data))
    return data


def vanilla_server_jar(mc_version: str) -> Path:
    """Download the vanilla server jar for the given MC version, sha1-checked."""
    info = _per_version_manifest(mc_version)
    srv = info["downloads"]["server"]
    dest = vanilla_server_dir(mc_version) / "server.jar"
    return _download(srv["url"], dest, expected_sha1=srv["sha1"])


def vanilla_client_jar(mc_version: str, dest: Path) -> Path:
    """Download a vanilla client jar to `dest`. Used by gen-palette tests.

    Note: the mcmap CLI itself has a `download-client` command that we test
    separately; this function exists so palette-test fixtures don't depend
    on that command's correctness.
    """
    info = _per_version_manifest(mc_version)
    clt = info["downloads"]["client"]
    dest.parent.mkdir(parents=True, exist_ok=True)
    return _download(clt["url"], dest, expected_sha1=clt["sha1"])


def java_major_for(mc_version: str) -> int:
    info = _per_version_manifest(mc_version)
    return int(info.get("javaVersion", {}).get("majorVersion", 8))


# ---------------------------------------------------------------------------
# JDK
# ---------------------------------------------------------------------------


def _adoptium_url(major: int) -> str:
    return (
        f"https://api.adoptium.net/v3/binary/latest/{major}/ga/linux/x64/"
        "jdk/hotspot/normal/eclipse"
    )


def _microsoft_jdk_url(major: int) -> str:
    return f"https://aka.ms/download-jdk/microsoft-jdk-{major}-linux-x64.tar.gz"


def jdk_home(major: int) -> Path:
    """Return the JAVA_HOME for the requested JDK major version.

    Tries Adoptium Temurin first; on 404 (or any non-tar response) falls
    back to Microsoft OpenJDK (which Mojang itself bundles for newer
    versions). Idempotent — extracts only on first call.
    """
    base = jdk_dir(major)
    marker = base / ".extracted"
    if marker.exists():
        # Re-resolve the extracted JAVA_HOME from the marker file.
        return Path(marker.read_text().strip())

    archive = base / "jdk.tar.gz"
    last_error: Exception | None = None
    for url_fn, source in (
        (_adoptium_url, "adoptium"),
        (_microsoft_jdk_url, "microsoft"),
    ):
        url = url_fn(major)
        try:
            log.info("Trying JDK %d from %s: %s", major, source, url)
            with _http_get(url, stream=True) as r:
                with archive.open("wb") as f:
                    for chunk in r.iter_content(chunk_size=1 << 20):
                        f.write(chunk)
            # Extract.
            with tarfile.open(archive, "r:gz") as tf:
                tf.extractall(base)
            archive.unlink()
            # The tarball contains a single top-level dir; locate it.
            top_dirs = [p for p in base.iterdir() if p.is_dir() and p.name != ".extracted"]
            if not top_dirs:
                raise RuntimeError(f"JDK tarball from {source} had no top-level dir")
            java_home = top_dirs[0]
            if not (java_home / "bin" / "java").exists():
                raise RuntimeError(f"extracted JDK has no bin/java: {java_home}")
            marker.write_text(str(java_home))
            return java_home
        except Exception as e:
            last_error = e
            log.warning("JDK fetch from %s failed: %s", source, e)
            if archive.exists():
                archive.unlink()
            # Clean partially extracted dirs before falling back.
            for p in base.iterdir():
                if p.is_dir():
                    shutil.rmtree(p)
    raise RuntimeError(f"could not download JDK {major} from any source") from last_error


def java_bin(major: int) -> Path:
    return jdk_home(major) / "bin" / "java"


# ---------------------------------------------------------------------------
# Forge installer + install
# ---------------------------------------------------------------------------


def _jvm_proxy_opts(env: dict[str, str]) -> list[str]:
    """Translate HTTPS_PROXY / HTTP_PROXY env vars into JVM proxy properties.

    The JVM's URL/HTTPS client doesn't read those env vars on its own — it
    needs `-Dhttp(s).proxyHost`/`-Dhttp(s).proxyPort` system properties. This
    matters when running behind a non-transparent local proxy (e.g. Clash on
    localhost:7890), where the system's own routing would otherwise mangle
    the JVM's TLS handshakes.
    """
    from urllib.parse import urlparse

    opts: list[str] = []
    for env_key, jvm_prefix in (("HTTPS_PROXY", "https"), ("HTTP_PROXY", "http")):
        raw = env.get(env_key) or env.get(env_key.lower())
        if not raw:
            continue
        parsed = urlparse(raw)
        host = parsed.hostname
        port = parsed.port
        if not host or not port:
            continue
        opts.append(f"-D{jvm_prefix}.proxyHost={host}")
        opts.append(f"-D{jvm_prefix}.proxyPort={port}")
    return opts


def forge_installer_jar(mc_version: str) -> Path:
    forge_ver = FORGE_BUILDS[mc_version]
    installer_name = f"forge-{mc_version}-{forge_ver}-installer.jar"
    url = (
        f"https://maven.minecraftforge.net/net/minecraftforge/forge/"
        f"{mc_version}-{forge_ver}/{installer_name}"
    )
    dest = forge_dir(mc_version) / installer_name
    return _download(url, dest)


def forge_install(mc_version: str, java_bin_path: Path) -> Path:
    """Run the installer's --installServer flag in a stable target dir.

    Idempotent: returns immediately if a forge-*.jar (non-installer) already
    exists in the install dir.

    Pre-stages our own copy of the vanilla server jar into the install dir
    so the Forge installer skips its own download (the installer reuses an
    existing `minecraft_server.<mc>.jar` if present). Mirror-list / version-
    manifest fetches are still made by the installer; we retry the whole
    invocation a few times with extended JVM network timeouts to absorb
    flaky upstream connectivity to launchermeta.mojang.com / Forge maven.
    """
    installer = forge_installer_jar(mc_version)
    target = forge_dir(mc_version) / "installed"
    target.mkdir(parents=True, exist_ok=True)
    # Detect already-installed.
    candidates = sorted(
        p for p in target.glob("forge-*.jar")
        if "installer" not in p.name and not p.name.endswith(".log")
    )
    if candidates:
        return target

    # Pre-stage the vanilla jar so the installer skips that download step.
    pre_staged = target / f"minecraft_server.{mc_version}.jar"
    if not pre_staged.exists():
        try:
            shutil.copy2(vanilla_server_jar(mc_version), pre_staged)
        except Exception as e:
            log.warning("Could not pre-stage vanilla server jar: %s", e)

    env = os.environ.copy()
    # Headless + long socket timeouts (60s connect, 180s read) so the
    # mirror-list and Mojang manifest fetches survive slow links.
    jvm_opts = [
        "-Djava.awt.headless=true",
        "-Dsun.net.client.defaultConnectTimeout=60000",
        "-Dsun.net.client.defaultReadTimeout=180000",
    ]
    jvm_opts.extend(_jvm_proxy_opts(env))
    env["JAVA_TOOL_OPTIONS"] = " ".join(jvm_opts)

    last_err: str = ""
    for attempt in range(1, 4):
        log.info(
            "Running forge installer for %s in %s (attempt %d/3)",
            mc_version, target, attempt,
        )
        proc = subprocess.run(
            [str(java_bin_path), "-jar", str(installer), "--installServer"],
            cwd=str(target),
            env=env,
            capture_output=True,
            text=True,
            timeout=900,
        )
        if proc.returncode == 0:
            return target
        last_err = (
            f"forge installer failed (exit={proc.returncode})\n"
            f"stdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )
        log.warning("Forge install attempt %d failed; retrying", attempt)

    raise RuntimeError(last_err)


def forge_boot_jar(install_dir: Path) -> Path:
    """Locate the bootable Forge jar inside an installed Forge directory.

    1.7.10 produces forge-...-universal.jar.
    1.12.2 produces forge-<mc>-<forge>.jar (no -universal suffix).
    """
    candidates = sorted(
        p for p in install_dir.glob("forge-*.jar")
        if "installer" not in p.name and not p.name.endswith(".log")
    )
    if not candidates:
        raise RuntimeError(f"no forge-*.jar found in {install_dir}")
    # Prefer -universal if present.
    for p in candidates:
        if "universal" in p.name:
            return p
    return candidates[0]


# ---------------------------------------------------------------------------
# Mods
# ---------------------------------------------------------------------------


def _github_release_jar(spec: dict, dest: Path) -> Path:
    import re
    api = f"https://api.github.com/repos/{spec['repo']}/releases/latest"
    headers = {}
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    rel = _http_get(api, headers=headers).json()
    pat = re.compile(spec["name_re"])
    for asset in rel.get("assets", []):
        if pat.match(asset["name"]):
            return _download(asset["browser_download_url"], dest)
    raise RuntimeError(
        f"no asset matching {spec['name_re']!r} in latest release of {spec['repo']}"
    )


def _modrinth_jar(spec: dict, dest: Path) -> Path:
    api = f"https://api.modrinth.com/v2/project/{spec['project']}/version"
    versions = _http_get(api).json()
    for v in versions:
        if (
            spec["game_version"] in v.get("game_versions", [])
            and spec["loader"] in v.get("loaders", [])
        ):
            files = v.get("files", [])
            if files:
                return _download(files[0]["url"], dest)
    raise RuntimeError(
        f"no Modrinth version of {spec['project']} for "
        f"{spec['game_version']}/{spec['loader']}"
    )


def mod_jar(key: str) -> Path:
    spec = MOD_SOURCES[key]
    dest = mod_dir() / f"{key}.jar"
    if dest.exists() and dest.stat().st_size > 0:
        return dest
    if spec["kind"] == "direct-url":
        return _download(spec["url"], dest)
    if spec["kind"] == "github-release":
        return _github_release_jar(spec, dest)
    if spec["kind"] == "modrinth":
        return _modrinth_jar(spec, dest)
    raise RuntimeError(f"unknown mod source kind: {spec['kind']}")
