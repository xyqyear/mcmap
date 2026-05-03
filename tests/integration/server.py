"""Boot, drive, and stop a Minecraft server for integration tests.

ServerInstance owns a single per-test world directory, a JVM subprocess, and
an mcrcon connection. Use as a context manager:

    with ServerInstance(flavor, work_dir) as srv:
        srv.rcon("setblock 0 64 0 minecraft:stone")
        # ...
    # On context exit, the server is saved + stopped cleanly.

Both first-run world generation and re-boots over the same world dir are
supported (caller decides whether `work_dir` is empty or pre-populated).
"""

from __future__ import annotations

import logging
import os
import re
import shutil
import socket
import subprocess
import threading
import time
from pathlib import Path

from mcrcon import MCRcon

from cache import (
    forge_boot_jar,
    forge_install,
    java_bin,
    mod_jar,
    vanilla_server_jar,
)
from flavors import Flavor

log = logging.getLogger(__name__)


# 1.7.10 prints `Done (Xs)! For help, type "help" or "?"`. 1.12.2 and modern
# print `Done (Xs)! For help, type "help"`. We only check up to the trailing
# `"help"` to cover both shapes.
BOOT_DONE_RE = re.compile(r'Done \([\d.]+s\)! For help, type "help"')
BOOT_TIMEOUT_SECONDS = 240
RCON_CONNECT_TIMEOUT_SECONDS = 60
STOP_TIMEOUT_SECONDS = 120
RCON_PASSWORD = "mcmap-test"


def _free_port() -> int:
    """Return an unused TCP port, racy but adequate for our test harness."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class ServerInstance:
    def __init__(self, flavor: Flavor, work_dir: Path):
        self.flavor = flavor
        self.work_dir = work_dir
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.rcon_port = _free_port()
        self.proc: subprocess.Popen[str] | None = None
        self.rcon: MCRcon | None = None
        self.log_path = work_dir / "server.log"
        self._log_buffer: list[str] = []
        self._log_lock = threading.Lock()
        self._reader_thread: threading.Thread | None = None

    # -- Context manager ----------------------------------------------------

    def __enter__(self) -> "ServerInstance":
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        # On exception, still try a clean stop. If that fails, fall back to
        # killing the JVM hard so test cleanup proceeds.
        try:
            self.save_and_stop()
        except Exception:
            log.exception("save_and_stop raised; killing JVM")
            self.kill()

    # -- Boot ---------------------------------------------------------------

    def _server_command(self) -> list[str]:
        java = str(java_bin(self.flavor.java_major))
        if self.flavor.distribution == "vanilla":
            jar = vanilla_server_jar(self.flavor.mc_version)
            # Copy the jar in to a stable name so the workdir is self-contained.
            local_jar = self.work_dir / "server.jar"
            if not local_jar.exists():
                shutil.copy2(jar, local_jar)
            return [java, "-Xms512M", "-Xmx2G", "-jar", str(local_jar), "nogui"]
        if self.flavor.distribution == "forge":
            install_dir = forge_install(self.flavor.mc_version, java_bin(self.flavor.java_major))
            # Symlink the static install artifacts (boot jar, vanilla jar,
            # libraries/) into the per-test work_dir. mods/, world/,
            # server.properties, eula.txt are written fresh below by start().
            # Symlinks within /tmp are cheap and avoid the ~200 MB copy that
            # libraries/ would otherwise incur per test.
            skip = {"mods", "world", "server.properties", "eula.txt", "logs", "crash-reports"}
            for entry in install_dir.iterdir():
                if entry.name in skip:
                    continue
                target = self.work_dir / entry.name
                if target.exists() or target.is_symlink():
                    continue
                target.symlink_to(entry)
            boot = forge_boot_jar(self.work_dir)
            return [java, "-Xms512M", "-Xmx2G", "-jar", str(boot), "nogui"]
        raise RuntimeError(f"unknown distribution: {self.flavor.distribution}")

    def _write_eula(self) -> None:
        (self.work_dir / "eula.txt").write_text("eula=true\n")

    def _write_server_properties(self) -> None:
        props = {
            "online-mode": "false",
            "enable-rcon": "true",
            "rcon.password": RCON_PASSWORD,
            "rcon.port": str(self.rcon_port),
            "server-port": str(_free_port()),
            "view-distance": "3",
            "spawn-protection": "0",
            "gamemode": "1",
            "default-gamemode": "creative",
            "difficulty": "peaceful",
            "spawn-monsters": "false",
            "spawn-animals": "false",
            "spawn-npcs": "false",
            "generate-structures": "false",
            "level-seed": "mcmap-test",
            "level-type": self.flavor.level_type,
            "generator-settings": self.flavor.generator_settings,
            "max-players": "2",
            "level-name": "world",
            "allow-nether": "false",
            "snooper-enabled": "false",
            "broadcast-rcon-to-ops": "false",
        }
        body = "\n".join(f"{k}={v}" for k, v in props.items()) + "\n"
        (self.work_dir / "server.properties").write_text(body)

    def _install_mods(self) -> None:
        if not self.flavor.mods:
            return
        mods_dir = self.work_dir / "mods"
        mods_dir.mkdir(exist_ok=True)
        for mod in self.flavor.mods:
            src = mod_jar(mod.key)
            shutil.copy2(src, mods_dir / mod.filename)

    def _ensure_legacy_origin_spawn(self) -> None:
        """Pre-boot 1.7.10/1.12.2 worlds once to relocate spawn to (0, 4, 0).

        Why: legacy /setblock and /testforblock both refuse to operate on
        unloaded chunks ("Cannot place block outside of the world"), and
        only the 17x17 spawn-chunks region around the world's spawn point
        is kept loaded with no players online. The vanilla spawn-finder
        with our fixed seed lands far from origin (e.g. chunk (-49, 95)),
        so chunk (0, 0) and friends are never loaded for tests.

        We can't /forceload (1.13.1+ only), and /setworldspawn doesn't
        retroactively load chunks within the same boot. So we do a one-
        shot pre-boot per work_dir: boot, /setworldspawn 0 4 0, save+stop.
        Subsequent boots in the same work_dir then load chunks (-8..8)
        around (0, 0) automatically, which is what the tests need.
        """
        if self.flavor.mc_version not in ("1.7.10", "1.12.2"):
            return
        if (self.work_dir / "world" / "level.dat").exists():
            return
        log.info("Pre-booting %s in %s to relocate spawn to (0, 4, 0)",
                 self.flavor.id, self.work_dir)
        cmd = self._server_command()
        log_fh = (self.work_dir / "preboot.log").open(
            "w", encoding="utf-8", errors="replace"
        )
        proc = subprocess.Popen(
            cmd,
            cwd=str(self.work_dir),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env={**os.environ, "JAVA_TOOL_OPTIONS": "-Djava.awt.headless=true"},
        )
        # Wait for boot, send commands via RCON, then stop.
        try:
            self._wait_for_boot_in(proc, log_fh)
            rcon = MCRcon("127.0.0.1", RCON_PASSWORD,
                          port=self.rcon_port, timeout=10)
            deadline = time.monotonic() + RCON_CONNECT_TIMEOUT_SECONDS
            while True:
                try:
                    rcon.connect()
                    break
                except Exception:
                    if time.monotonic() > deadline:
                        raise
                    time.sleep(0.5)
            rcon.command("setworldspawn 0 4 0")
            rcon.command("save-off")
            rcon.command("save-all")
            rcon.command("stop")
            rcon.disconnect()
            try:
                proc.wait(timeout=STOP_TIMEOUT_SECONDS)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=10)
        finally:
            log_fh.close()
        # Re-pick a free RCON port for the real boot — the JVM may not
        # have released the prior one in time.
        self.rcon_port = _free_port()
        self._write_server_properties()

    def _wait_for_boot_in(
        self, proc: subprocess.Popen[str], log_fh
    ) -> None:
        """Stream `proc` stdout to log_fh until BOOT_DONE_RE matches."""
        assert proc.stdout is not None
        deadline = time.monotonic() + BOOT_TIMEOUT_SECONDS
        for line in proc.stdout:
            log_fh.write(line)
            log_fh.flush()
            if BOOT_DONE_RE.search(line):
                return
            if time.monotonic() > deadline:
                raise TimeoutError(
                    f"pre-boot did not reach 'Done' within {BOOT_TIMEOUT_SECONDS}s"
                )

    def start(self) -> None:
        if self.proc is not None:
            raise RuntimeError("server already started")
        self._write_eula()
        self._write_server_properties()
        self._install_mods()
        self._ensure_legacy_origin_spawn()
        cmd = self._server_command()
        log.info("Booting %s in %s (rcon port %d)", self.flavor.id, self.work_dir, self.rcon_port)
        log_fh = self.log_path.open("w", encoding="utf-8", errors="replace")
        self.proc = subprocess.Popen(
            cmd,
            cwd=str(self.work_dir),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env={**os.environ, "JAVA_TOOL_OPTIONS": "-Djava.awt.headless=true"},
        )

        def reader() -> None:
            assert self.proc is not None and self.proc.stdout is not None
            for line in self.proc.stdout:
                with self._log_lock:
                    self._log_buffer.append(line)
                log_fh.write(line)
                log_fh.flush()
            log_fh.close()

        self._reader_thread = threading.Thread(target=reader, daemon=True)
        self._reader_thread.start()
        self._wait_for_boot()
        self._connect_rcon()

    def _wait_for_boot(self) -> None:
        deadline = time.monotonic() + BOOT_TIMEOUT_SECONDS
        last_idx = 0
        while time.monotonic() < deadline:
            if self.proc and self.proc.poll() is not None:
                tail = self._tail_log(40)
                raise RuntimeError(
                    f"server JVM exited during boot (rc={self.proc.returncode}):\n{tail}"
                )
            with self._log_lock:
                snapshot = self._log_buffer[last_idx:]
                last_idx = len(self._log_buffer)
            for line in snapshot:
                if BOOT_DONE_RE.search(line):
                    log.info("server boot done: %s", line.strip())
                    return
            time.sleep(0.5)
        tail = self._tail_log(40)
        raise TimeoutError(f"server failed to reach 'Done' within {BOOT_TIMEOUT_SECONDS}s:\n{tail}")

    def _connect_rcon(self) -> None:
        deadline = time.monotonic() + RCON_CONNECT_TIMEOUT_SECONDS
        last_err: Exception | None = None
        while time.monotonic() < deadline:
            try:
                rcon = MCRcon("127.0.0.1", RCON_PASSWORD, port=self.rcon_port, timeout=10)
                rcon.connect()
                self.rcon = rcon
                return
            except Exception as e:
                last_err = e
                time.sleep(0.5)
        raise RuntimeError(f"failed to connect to RCON on port {self.rcon_port}: {last_err}")

    # -- Commands -----------------------------------------------------------

    def cmd(self, command: str) -> str:
        """Send a single command. Returns the server's reply."""
        if self.rcon is None:
            raise RuntimeError("RCON not connected")
        log.debug("RCON> %s", command)
        reply = self.rcon.command(command)
        log.debug("RCON< %s", reply)
        return reply

    def _ensure_chunk_loaded(self, x: int, z: int) -> None:
        """Force-load the chunk containing block (x, _, z).

        Modern (1.13+) doesn't auto-load chunks for /setblock — recent
        versions reply "That position is not loaded" if the target chunk
        isn't held by spawn-chunks or a forceload ticket. We forceload
        explicitly. Legacy 1.7.10 and 1.12.2 keep a 19x19 spawn area
        loaded automatically (and have no forceload command), so we skip.
        """
        if self.flavor.mc_version in ("1.7.10", "1.12.2"):
            return
        # `forceload add` accepts block-precision coords and operates on the
        # containing chunk.
        self.cmd(f"forceload add {x} {z}")

    def setblock(self, x: int, y: int, z: int, block: str, *, data: int | None = None) -> str:
        """/setblock x y z block — version-aware.

        For 1.7/1.12 the syntax accepts an optional integer data value as the
        4th positional arg. Modern uses blockstate brackets, but we don't need
        those for the kinds of blocks the tests place.
        """
        self._ensure_chunk_loaded(x, z)
        if self.flavor.mc_version in ("1.7.10", "1.12.2"):
            tail = "" if data is None else f" {data}"
            return self.cmd(f"setblock {x} {y} {z} {block}{tail}")
        return self.cmd(f"setblock {x} {y} {z} {block}")

    def assert_block(self, x: int, y: int, z: int, block: str) -> None:
        """Assert that the block at (x,y,z) matches `block`. Version-aware.

        Legacy: /testforblock — mismatch reply contains "(expected:" substring.
        Modern: /execute store result + /scoreboard players get — read score.
        """
        if self.flavor.mc_version in ("1.7.10", "1.12.2"):
            reply = self.cmd(f"testforblock {x} {y} {z} {block}")
            if "(expected:" in reply:
                raise AssertionError(f"testforblock failed: {reply!r}")
            if "Successfully found the block" not in reply:
                raise AssertionError(f"unexpected testforblock reply: {reply!r}")
            return
        # Modern: use a scoreboard objective that we lazily create. Force-load
        # so the read isn't rejected on an unloaded chunk.
        self._ensure_chunk_loaded(x, z)
        self.cmd("scoreboard objectives add mcmap_test dummy")
        self.cmd("scoreboard players set check mcmap_test 0")
        self.cmd(
            f"execute store success score check mcmap_test "
            f"if block {x} {y} {z} {block}"
        )
        reply = self.cmd("scoreboard players get check mcmap_test")
        if " 1 " not in f" {reply} ":
            raise AssertionError(
                f"block at ({x},{y},{z}) is not {block} (scoreboard reply: {reply!r})"
            )

    def block_at_is(self, x: int, y: int, z: int, block: str) -> bool:
        """Like assert_block but returns bool instead of raising."""
        try:
            self.assert_block(x, y, z, block)
            return True
        except AssertionError:
            return False

    # -- Save / stop --------------------------------------------------------

    def save_and_stop(self) -> None:
        if self.proc is None:
            return
        try:
            if self.rcon is not None:
                # Flush sequence depends on version. /save-all flush exists
                # only in 1.13+. For legacy we use save-off + save-all and
                # rely on /stop's clean-shutdown sync flush.
                if self.flavor.mc_version in ("1.7.10", "1.12.2"):
                    try:
                        self.cmd("save-off")
                        self.cmd("save-all")
                    except Exception:
                        log.exception("save sequence raised; continuing to stop")
                else:
                    try:
                        self.cmd("save-all flush")
                    except Exception:
                        log.exception("save-all flush raised; continuing to stop")
                try:
                    self.cmd("stop")
                except Exception:
                    pass
                try:
                    self.rcon.disconnect()
                except Exception:
                    pass
                self.rcon = None
            self.proc.wait(timeout=STOP_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            log.warning("server did not stop in %ds; killing", STOP_TIMEOUT_SECONDS)
            self.kill()
        finally:
            self.proc = None
            if self._reader_thread is not None:
                self._reader_thread.join(timeout=10)
                self._reader_thread = None

    def kill(self) -> None:
        if self.rcon is not None:
            try:
                self.rcon.disconnect()
            except Exception:
                pass
            self.rcon = None
        if self.proc is not None:
            try:
                self.proc.kill()
                self.proc.wait(timeout=30)
            except Exception:
                pass
            self.proc = None

    # -- Log access ---------------------------------------------------------

    def _tail_log(self, n: int) -> str:
        with self._log_lock:
            return "".join(self._log_buffer[-n:])

    def full_log(self) -> str:
        with self._log_lock:
            return "".join(self._log_buffer)
