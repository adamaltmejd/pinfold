#!/bin/sh
set -eu

# The firewall exception names one path, not Cargo's changing binary hash.
runner="$HOME/.local/lib/pinfold-ci/e2e"
mkdir -p "$(dirname "$runner")"
cp "$1" "$runner"
shift
exec "$runner" "$@"
