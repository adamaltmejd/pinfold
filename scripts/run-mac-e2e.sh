#!/bin/sh
set -eu

# The firewall exception names one path, not Cargo's changing binary hash.
runner="$HOME/.local/lib/pinfold-ci/e2e"
mkdir -p "$(dirname "$runner")"
# Replace, never overwrite: macOS caches a binary's code signature per inode
# and SIGKILLs an exec of changed contents under the same inode.
cp "$1" "$runner.$$"
mv -f "$runner.$$" "$runner"
shift
exec "$runner" "$@"
