#!/bin/sh
# Usage: env.sh CONTAINER. Prints the environment the probe ran in.
sw_vers; uname -m
echo "host uid:gid $(id -u):$(id -g)"
container --version
container system version 2>/dev/null || true
echo "host $(git --version)"
python3 --version
starship --version | head -1
zed --version
container image list | grep 'profile-default *latest'
container exec "$1" sh -c 'echo "guest $(git --version)"; uname -sr; uname -m; id; grep /work /proc/mounts'
echo "host git global config keys (core/status/index/feature):"
git config --global --get-regexp '^(core|status|index|feature)\.' | grep -v pager
