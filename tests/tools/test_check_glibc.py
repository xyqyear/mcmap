import os
from pathlib import Path
import subprocess
import sys

import pytest


pytestmark = pytest.mark.skipif(sys.platform != "linux", reason="GNU ELF release check")
CHECK = Path(__file__).resolve().parents[2] / "scripts" / "check-glibc.sh"


def run_check(tmp_path, version_info, *, readelf_status=0, diagnostic=""):
    fixture = tmp_path / "version-info.txt"
    fixture.write_text(version_info, encoding="utf-8")
    readelf = tmp_path / "readelf"
    readelf.write_text(
        '#!/usr/bin/env bash\n'
        'cat "$TEST_VERSION_INFO"\n'
        'printf "%s" "$TEST_DIAGNOSTIC" >&2\n'
        'exit "$TEST_READELF_STATUS"\n',
        encoding="utf-8",
    )
    readelf.chmod(0o755)
    env = {**os.environ, "PATH": f"{tmp_path}:{os.environ['PATH']}",
           "TEST_VERSION_INFO": str(fixture), "TEST_DIAGNOSTIC": diagnostic,
           "TEST_READELF_STATUS": str(readelf_status)}
    return subprocess.run(["bash", str(CHECK), "unused-test-elf"], env=env,
                          capture_output=True, text=True, check=False)


def needs(*names):
    return "Version needs section '.gnu.version_r' contains 1 entry:\n" + "".join(
        f"  0x0010:   Name: {name}  Flags: none  Version: 2\n" for name in names
    )


def test_checks_highest_requirement_with_numeric_components(tmp_path):
    result = run_check(tmp_path, needs("GLIBC_2.9", "GLIBC_2.34", "GLIBC_2.2.5"))
    assert result.returncode == 0
    assert "Maximum required GLIBC version: GLIBC_2.34" in result.stdout


@pytest.mark.parametrize("version", ["GLIBC_2.35", "GLIBC_2.38", "GLIBC_2.100", "GLIBC_2.34.1", "GLIBC_3.0"])
def test_rejects_every_requirement_above_the_baseline(tmp_path, version):
    result = run_check(tmp_path, needs(version, "GLIBC_2.2.5", "GLIBC_2.34"))
    assert result.returncode != 0
    assert f"Maximum required GLIBC version: {version}" in result.stdout


@pytest.mark.parametrize("version_info", [
    "", "No version information found in this file.\n", needs("GCC_3.0"),
    "Version definition section '.gnu.version_d' contains 1 entry:\n  Name: GLIBC_2.34\n",
    needs("GLIBC_PRIVATE", "GLIBC_2.34"),
    needs("GLIBC_ABI_DT_RELR", "GLIBC_2.34"),
])
def test_does_not_accept_absent_or_unrecognized_requirements(tmp_path, version_info):
    assert run_check(tmp_path, version_info).returncode != 0


@pytest.mark.parametrize("status,diagnostic", [(1, ""), (0, "readelf: Warning: corrupt section\n")])
def test_rejects_readelf_failures_even_with_acceptable_partial_output(tmp_path, status, diagnostic):
    result = run_check(tmp_path, needs("GLIBC_2.34"), readelf_status=status, diagnostic=diagnostic)
    assert result.returncode != 0


def test_actual_readelf_rejects_non_elf_and_missing_files(tmp_path):
    invalid = tmp_path / "not-an-elf"
    invalid.write_text("GLIBC_2.34\n", encoding="utf-8")
    for path in [invalid, tmp_path / "missing"]:
        result = subprocess.run(["bash", str(CHECK), str(path)], capture_output=True, check=False)
        assert result.returncode != 0
