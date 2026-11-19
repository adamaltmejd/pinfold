#!/bin/bash
# Usage: run.sh. Starts the box, runs the case into logs/, removes the box
# and the temp directory.
cd "$(dirname "$0")" || exit 1
ROOT=$(mktemp -d /tmp/pfs.XXXX)
CT=pfs-hc-${ROOT##*.}
IMG=localhost/pinfold/profile-default:latest
container run -d --name "$CT" --user 501:20 -v "$ROOT":/work --entrypoint sleep $IMG infinity >/dev/null || {
  rm -rf "$ROOT"; exit 1
}
# Only a box this run created is removed.
trap 'container rm -f "$CT" >/dev/null; rm -rf "$ROOT"' EXIT
{
  sw_vers; uname -m
  container --version
  echo "host $(git --version)"
  container image list | grep 'profile-default *latest'
  container exec "$CT" sh -c 'echo "guest $(git --version)"; uname -sr; id; grep /work /proc/mounts'
  echo "box: container run -d --name $CT --user 501:20 -v $ROOT:/work --entrypoint sleep $IMG infinity"
} > logs/env.log 2>&1
PFS_CT=$CT PFS_ROOT=$ROOT python3 hc.py 3 30 logs/hc.jsonl >/dev/null
echo done
