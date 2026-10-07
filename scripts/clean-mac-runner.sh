#!/bin/sh
# For the dedicated CI runtime only. Run while no build or suite is active.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temp=${TMPDIR:-$(getconf DARWIN_USER_TEMP_DIR)}
cache="$temp/pinfold-e2e-cache"
# An interrupted job can leave running boxes. Only the shared builder survives.
containers=$(container list --all --quiet)
for container_id in $containers; do
  if [ "$container_id" != buildkit ]; then
    container delete --force "$container_id"
  fi
done
# Removing the idle default profile also prevents testing yesterday's pins.
container image prune --all >/dev/null
builder=$(container builder status --quiet)
if [ -n "$builder" ]; then
  container exec buildkit buildctl prune --all --keep-storage 4096 >/dev/null
fi

# The binary recreates its embedded init cheaply. Current harnesses stay cached.
rm -rf "$cache/pinfold/artifacts/init"
if [ -d "$cache" ]; then
  cache_kb=$(du -sk "$cache" | awk '{print $1}')
  if [ "$cache_kb" -gt 2097152 ]; then
    echo 'Test artifact cache exceeds 2 GiB; resetting it.'
    rm -rf "$cache"
  fi
fi

container system df
df -h "$root"
