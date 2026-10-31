#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# != 1 ]]; then
    echo "Usage: bash scripts/check-glibc.sh <GNU ELF binary>" >&2
    exit 1
fi

diagnostics=$(mktemp)
trap 'rm -f -- "$diagnostics"' EXIT
if ! version_info=$(readelf --version-info --wide -- "$1" 2>"$diagnostics"); then
    cat "$diagnostics" >&2
    echo "Cannot read ELF version requirements: $1" >&2
    exit 1
fi
if [[ -s "$diagnostics" ]]; then
    cat "$diagnostics" >&2
    echo "Refusing incomplete ELF version information: $1" >&2
    exit 1
fi

awk -v limit=2.34 '
function newer(left, right, a, b, na, nb, i) {
    na = split(left, a, ".")
    nb = split(right, b, ".")
    for (i = 1; i <= na || i <= nb; i++) {
        if (a[i] + 0 > b[i] + 0) return 1
        if (a[i] + 0 < b[i] + 0) return 0
    }
    return 0
}
/^Version .* section / { needs = 0 }
/^Version needs section / { needs = 1 }
needs {
    for (i = 1; i < NF; i++) {
        if ($i == "Name:" && $(i + 1) ~ /^GLIBC_/) {
            name = $(i + 1)
            if (name !~ /^GLIBC_[0-9]+(\.[0-9]+)+$/) {
                print "Unsupported GLIBC requirement: " name > "/dev/stderr"
                invalid = 1
                continue
            }
            version = substr(name, 7)
            if (!found || newer(version, maximum)) maximum = version
            found = 1
        }
    }
}
END {
    if (!found) {
        print "No GLIBC version requirements found in ELF" > "/dev/stderr"
        exit 1
    }
    print "Maximum required GLIBC version: GLIBC_" maximum " (allowed: GLIBC_" limit ")"
    if (invalid || newer(maximum, limit)) {
        print "GNU binary exceeds the supported GLIBC baseline" > "/dev/stderr"
        exit 1
    }
}
' <<< "$version_info"
