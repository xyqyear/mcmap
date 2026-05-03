#!/usr/bin/env bash
# Run the integration suite locally with the dev proxy configured.
#
# Why this exists: the JVM (used to install Forge) and uv (used to fetch
# Python deps) both ignore the host's transparent TUN proxy when present,
# so we set explicit HTTP(S)_PROXY env vars pointing at the local Clash
# instance on :7890. The Forge installer translates these into JVM
# system properties via the cache.py helper.
#
# Usage:
#   ./run-local.sh                      # run all flavors
#   ./run-local.sh vanilla-latest       # one flavor only
#   ./run-local.sh forge-1.7.10-neid -k render  # plus extra pytest args
#
# Extra args after the flavor are passed through to pytest unchanged.

set -euo pipefail

cd "$(dirname "$0")"

export MCMAP_INTEGRATION_TESTS=1
export HTTP_PROXY="${HTTP_PROXY:-http://localhost:7890}"
export HTTPS_PROXY="${HTTPS_PROXY:-http://localhost:7890}"

if [[ $# -ge 1 ]] && [[ "$1" != -* ]]; then
    export MCMAP_FLAVOR="$1"
    shift
fi

exec uv run pytest -ra "$@"
