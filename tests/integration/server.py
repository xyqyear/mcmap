"""Boot, drive, and stop a Minecraft server for integration tests.

Commands flow over the JVM's stdin (no RCON listener); replies are sliced
out of the server log via a per-call sentinel marker. Use as a context
manager:

    with ServerInstance(flavor, work_dir) as srv:
        srv.setblock(0, 64, 0, "minecraft:stone")
        srv.assert_block(0, 64, 0, "minecraft:stone")
    # On context exit, the server is saved and stopped cleanly.

Both first-run world generation and re-boots over the same world dir are
supported. Legacy worlds (1.7.10, 1.12.2) get a one-shot pre-boot the first
time their work_dir is used so /setblock has loaded chunks at origin to
operate on; the pre-boot is transparent to callers.
"""

from __future__ import annotations

import itertools
import logging
import os
import re
import shutil
import socket
import subprocess
import threading
import time
from pathlib import Path

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
# print `Done (Xs)! For help, type "help"`. Match up to the trailing `"help"`
# to cover both shapes.
BOOT_DONE_RE = re.compile(r'Done \([\d.]+s\)! For help, type "help"')
BOOT_TIMEOUT_SECONDS = 240
COMMAND_TIMEOUT_SECONDS = 60
STOP_TIMEOUT_SECONDS = 120
LEGACY_VERSIONS = frozenset({"1.7.10", "1.12.2"})


def _free_port() -> int:
    """Bind and immediately release a fresh ephemeral TCP port. Used only
    for the in-game `server-port` (Minecraft refuses to boot if it can't
    bind one).
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class ServerInstance:
    """A runnable Minecraft server with a stdin command channel.

    Single-threaded use is assumed: callers issue commands sequentially.
    Concurrent calls to `cmd()` are not safe (the reply-slicing assumes
    nothing else is interleaving commands), but the underlying log buffer
    and stdin write are guarded by a lock so the failure mode is wrong
    replies, not corruption.
    """

    def __init__(self, flavor: Flavor, work_dir: Path) -> None:
        self.flavor = flavor
        self.work_dir = work_dir
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = work_dir / "server.log"
        self.proc: subprocess.Popen[str] | None = None
        self.port: int | None = None
        self._log_buffer: list[str] = []
        self._lock = threading.Lock()
        self._cmd_seq = itertools.count()
        self._reader_thread: threading.Thread | None = None

    # -- Context manager ----------------------------------------------------

    def __enter__(self) -> "ServerInstance":
        self._maybe_preboot()
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        try:
            self.save_and_stop()
        except Exception:
            log.exception("save_and_stop raised; killing JVM")
            self.kill()

    # -- Boot orchestration -------------------------------------------------

    def _maybe_preboot(self) -> None:
        """Run a one-shot pre-boot for legacy worlds to relocate spawn to
        origin.

        Why: legacy /setblock and /testforblock both refuse to operate on
        unloaded chunks ("Cannot place block outside of the world"), and
        only the 17×17 spawn-chunks region around the world's spawn point
        is kept loaded with no players online. The vanilla spawn-finder
        with our fixed seed lands far from origin (e.g. chunk (-49, 95)),
        so chunks near (0, 0) are never loaded for tests.

        We can't /forceload (1.13.1+ only), and /setworldspawn doesn't
        retroactively load chunks within the same boot. So we do a one-
        shot pre-boot per work_dir: boot, /setworldspawn 0 4 0, save+stop.
        Subsequent boots in the same work_dir then load chunks (-8..8)
        around (0, 0) automatically.
        """
        if self.flavor.mc_version not in LEGACY_VERSIONS:
            return
        if (self.work_dir / "world" / "level.dat").exists():
            return
        log.info(
            "Pre-booting %s in %s to relocate spawn to (0, 4, 0)",
            self.flavor.id, self.work_dir,
        )
        self.start(log_filename="preboot.log")
        try:
            self.cmd("setworldspawn 0 4 0")
        finally:
            self.save_and_stop()

    def start(self, log_filename: str = "server.log") -> None:
        """Boot the JVM, attach the reader thread, wait for `Done`."""
        if self.proc is not None:
            raise RuntimeError("server already started")
        self._write_eula()
        self._write_server_properties()
        self._install_mods()
        cmd_argv = self._server_command()
        log.info(
            "Booting %s in %s (log -> %s)",
            self.flavor.id, self.work_dir, log_filename,
        )
        log_fh = (self.work_dir / log_filename).open(
            "w", encoding="utf-8", errors="replace"
        )
        # Reset per-boot state so markers don't collide with leftovers from
        # an earlier boot's log file (pre-boot then real boot share `self`).
        with self._lock:
            self._log_buffer = []
            self._cmd_seq = itertools.count()
        self.proc = subprocess.Popen(
            cmd_argv,
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
                with self._lock:
                    self._log_buffer.append(line)
                log_fh.write(line)
                log_fh.flush()
            log_fh.close()

        self._reader_thread = threading.Thread(target=reader, daemon=True)
        self._reader_thread.start()
        self._wait_for_boot()

    def _wait_for_boot(self) -> None:
        deadline = time.monotonic() + BOOT_TIMEOUT_SECONDS
        last_idx = 0
        while time.monotonic() < deadline:
            assert self.proc is not None
            if self.proc.poll() is not None:
                tail = self._tail_log(40)
                raise RuntimeError(
                    f"server JVM exited during boot (rc={self.proc.returncode}):\n{tail}"
                )
            with self._lock:
                snapshot = self._log_buffer[last_idx:]
                last_idx = len(self._log_buffer)
            for line in snapshot:
                if BOOT_DONE_RE.search(line):
                    log.info("server boot done: %s", line.strip())
                    return
            time.sleep(0.5)
        raise TimeoutError(
            f"server failed to reach 'Done' within {BOOT_TIMEOUT_SECONDS}s:\n"
            f"{self._tail_log(40)}"
        )

    # -- Server-side files --------------------------------------------------

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
            install_dir = forge_install(
                self.flavor.mc_version, java_bin(self.flavor.java_major)
            )
            # Symlink the static install artifacts (boot jar, vanilla jar,
            # libraries/) into the per-test work_dir. mods/, world/,
            # server.properties, eula.txt are written fresh by start().
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
        # No RCON: commands flow over stdin. server-port still has to be
        # bound by the JVM (Minecraft refuses to boot otherwise) so we pick
        # a free ephemeral port for it.
        self.port = _free_port()
        props = {
            "online-mode": "false",
            "server-port": str(self.port),
            "allow-flight": "true",
            "network-compression-threshold": "-1",
            "view-distance": "3",
            "simulation-distance": "3",
            "spawn-protection": "0",
            "gamemode": "1",
            "default-gamemode": "creative",
            "difficulty": "peaceful",
            "spawn-monsters": "false",
            "spawn-animals": "true",
            "spawn-npcs": "false",
            "generate-structures": "false",
            "level-seed": "mcmap-test",
            "level-type": self.flavor.level_type,
            "generator-settings": self.flavor.generator_settings,
            "max-players": "2",
            "level-name": "world",
            "allow-nether": "false",
            "snooper-enabled": "false",
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

    # -- Commands -----------------------------------------------------------

    def cmd(self, command: str) -> str:
        """Send a command; return everything the server logged in response.

        The reply is the slice of server stdout between the moment the
        command was sent and a sentinel marker emitted by `/say` after it.
        Minecraft processes stdin commands serially in its tick loop, so
        the marker line bounds the response unambiguously. Substring-match
        the returned text the same way you'd match an RCON reply.

        Don't pass `stop` — the JVM shuts down before the marker can be
        echoed and this would time out. `save_and_stop()` handles stop.
        """
        if self.proc is None or self.proc.stdin is None:
            raise RuntimeError("server not running")
        seq = next(self._cmd_seq)
        marker = f"__MCMAP_CMD_{seq}_DONE__"
        log.debug("CMD> %s", command)
        with self._lock:
            start = len(self._log_buffer)
            self.proc.stdin.write(f"{command}\nsay {marker}\n")
            self.proc.stdin.flush()
        deadline = time.monotonic() + COMMAND_TIMEOUT_SECONDS
        while True:
            with self._lock:
                end = len(self._log_buffer)
                for i in range(start, end):
                    if marker in self._log_buffer[i]:
                        reply = "".join(self._log_buffer[start:i])
                        log.debug("CMD< %s", reply.rstrip())
                        return reply
            if self.proc.poll() is not None:
                raise RuntimeError(
                    f"server JVM exited while awaiting reply to {command!r} "
                    f"(rc={self.proc.returncode})\n--- last 40 log lines ---\n"
                    f"{self._tail_log(40)}"
                )
            if time.monotonic() > deadline:
                raise TimeoutError(
                    f"command {command!r} did not complete within "
                    f"{COMMAND_TIMEOUT_SECONDS}s\n--- last 40 log lines ---\n"
                    f"{self._tail_log(40)}"
                )
            time.sleep(0.05)

    def _ensure_chunk_loaded(self, x: int, z: int) -> None:
        """Force-load the chunk containing (x, _, z) on modern flavors.

        Modern (1.13+) doesn't auto-load chunks for /setblock — recent
        versions reply "That position is not loaded" if the target chunk
        isn't held by spawn-chunks or a forceload ticket. Legacy 1.7.10
        and 1.12.2 keep a 19×19 spawn area loaded automatically (and have
        no forceload command), so we skip.
        """
        if self.flavor.mc_version in LEGACY_VERSIONS:
            return
        # `forceload add` accepts block-precision coords and operates on the
        # containing chunk.
        self.cmd(f"forceload add {x} {z}")

    def setblock(
        self, x: int, y: int, z: int, block: str, *, data: int | None = None
    ) -> str:
        """/setblock x y z block — version-aware.

        For 1.7/1.12 the syntax accepts an optional integer data value as
        the 4th positional arg. Modern uses blockstate brackets, but we
        don't need those for the kinds of blocks the tests place.
        """
        self._ensure_chunk_loaded(x, z)
        if self.flavor.mc_version in LEGACY_VERSIONS:
            tail = "" if data is None else f" {data}"
            return self.cmd(f"setblock {x} {y} {z} {block}{tail}")
        return self.cmd(f"setblock {x} {y} {z} {block}")

    def assert_block(self, x: int, y: int, z: int, block: str) -> None:
        """Assert that the block at (x, y, z) matches `block`. Version-aware.

        Legacy: /testforblock — mismatch reply contains "(expected:" substring.
        Modern: /execute store success + /scoreboard players get — read score.
        """
        if self.flavor.mc_version in LEGACY_VERSIONS:
            reply = self.cmd(f"testforblock {x} {y} {z} {block}")
            if "(expected:" in reply:
                raise AssertionError(f"testforblock failed: {reply!r}")
            if "Successfully found the block" not in reply:
                raise AssertionError(f"unexpected testforblock reply: {reply!r}")
            return
        # Modern: lazily-create scoreboard objective; force-load so the read
        # isn't rejected on an unloaded chunk.
        self._ensure_chunk_loaded(x, z)
        self.cmd("scoreboard objectives add mcmap_test dummy")
        self.cmd("scoreboard players set check mcmap_test 0")
        self.cmd(
            f"execute store success score check mcmap_test "
            f"if block {x} {y} {z} {block}"
        )
        reply = self.cmd("scoreboard players get check mcmap_test")
        # Reply line shape: `check has 1 [mcmap_test]` (1.13+) or
        # `check has 1 (mcmap_test)` (older), depending on version.
        if not re.search(r"\bhas\s+1\b", reply):
            raise AssertionError(
                f"block at ({x},{y},{z}) is not {block} (reply: {reply!r})"
            )

    def block_at_is(self, x: int, y: int, z: int, block: str) -> bool:
        """Like assert_block but returns bool instead of raising."""
        try:
            self.assert_block(x, y, z, block)
            return True
        except AssertionError:
            return False

    def teleport_player(self, player: str, x: float, y: float, z: float) -> str:
        return self.cmd(f"tp {player} {x:g} {y:g} {z:g}")

    def set_player_survival(self, player: str) -> str:
        if self.flavor.mc_version in LEGACY_VERSIONS:
            return self.cmd(f"gamemode 0 {player}")
        return self.cmd(f"gamemode survival {player}")

    def gametime(self) -> int:
        reply = self.cmd("time query gametime")
        match = re.search(r"\b(?:game\s+)?time\s+is\s+(-?\d+)\b", reply, re.IGNORECASE)
        if match is None:
            raise AssertionError(f"could not parse gametime from reply: {reply!r}")
        return int(match.group(1))

    def wait_ticks(self, ticks: int) -> None:
        try:
            start = self.gametime()
        except AssertionError:
            time.sleep(ticks / 20.0)
            return
        target = start + ticks
        deadline = time.monotonic() + max(30.0, ticks / 20.0 + 20.0)
        while time.monotonic() < deadline:
            if self.gametime() >= target:
                return
            time.sleep(0.1)
        raise TimeoutError(f"server gametime did not advance by {ticks} ticks")

    # -- Save / stop --------------------------------------------------------

    def save_and_stop(self) -> None:
        if self.proc is None:
            return
        try:
            try:
                if self.flavor.mc_version in LEGACY_VERSIONS:
                    # /save-all flush exists only in 1.13+. For legacy we
                    # use save-off + save-all and rely on /stop's clean
                    # shutdown to flush pending writes.
                    self.cmd("save-off")
                    self.cmd("save-all")
                else:
                    self.cmd("save-all flush")
            except Exception:
                log.exception("save sequence raised; continuing to stop")
            # `stop` triggers shutdown immediately, so the JVM won't get to
            # echo a sentinel marker. Bypass cmd() and write directly.
            try:
                with self._lock:
                    if self.proc.stdin is not None:
                        self.proc.stdin.write("stop\n")
                        self.proc.stdin.flush()
            except Exception:
                pass
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
        if self.proc is not None:
            try:
                self.proc.kill()
                self.proc.wait(timeout=30)
            except Exception:
                pass
            self.proc = None

    # -- Log access ---------------------------------------------------------

    def _tail_log(self, n: int) -> str:
        with self._lock:
            return "".join(self._log_buffer[-n:])

    def full_log(self) -> str:
        with self._lock:
            return "".join(self._log_buffer)
