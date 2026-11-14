#!/bin/bash
# Linux cases for #294. Usage: linux-cases.sh PATH/TO/pinfold
# Uses its own XDG config and state roots under a short /tmp path (#303)
# and the real cache root. It uses no credentials.
set -u
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
W=$(mktemp -d /tmp/pfl.XXXX)
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
export PINFOLD_CPUS=2 PINFOLD_MEMORY=1G
mkdir -p "$XDG_STATE_HOME/pinfold"
touch "$XDG_STATE_HOME/pinfold/maintenance"
unset VISUAL EDITOR
pinfold() { "$BIN" "$@"; }
scrub() { sed -e "s#$W#\$W#g" -e "s#$HOME#~#g" -e "s#$(id -un)#user#g"; }
run() { echo "\$ $*"; "$@" </dev/null 2>&1 | tail -25 | scrub; echo "[exit ${PIPESTATUS[0]}]"; echo; }
count() { find "$W/config" "$W/state" -mindepth 1 2>/dev/null | wc -l; }
git init -q "$W/proj"
git -C "$W/proj" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
mkdir "$W/plain"

echo "## D. Outside Git: refused as not-git, nothing created"
cd "$W/plain"
run pinfold pi --version
EDITOR=true run pinfold config edit
run pinfold config
run pinfold build
echo "entries under config and state roots: $(count)"; echo

cd "$W/proj"
echo "## A. No configuration: refused"
run pinfold pi --version
echo "project state dirs: $(ls "$W/state/pinfold/projects" 2>/dev/null | wc -l)"; echo

echo "## B. Configured Git project starts (positive control)"
run pinfold profile new probe-linux294 --from default
export PINFOLD_PROFILE=probe-linux294
EDITOR=true run pinfold config edit
df -BG /
AVAILABLE=$(df -B1 --output=avail / | tail -1)
if (( AVAILABLE < 12 * 1024 * 1024 * 1024 )); then
    echo "under 12 GiB free; build not run"
    exit 1
fi
run pinfold build
run pinfold pi --version
run pinfold push
run pinfold fetch

echo "## E. mounts other than the template: refused as spec"
CFG=$(find "$W/config" -name pinfold.toml -path "*projects*" | head -1)
cp "$CFG" "$W/cfg.bak"
sed -i 's#^mounts = .*#mounts = [{ host = "/etc", kind = "share" }]#' "$CFG"
grep -n "^mounts" "$CFG" | scrub
run pinfold pi --version
cp "$W/cfg.bak" "$CFG"

echo "## C. host-path-in-project: refused; external roots run"
mkdir -p "$W/proj/inside"; ln -s "$W/proj/inside" "$W/cachelink"
XDG_CACHE_HOME=$W/cachelink/missing/cache run pinfold pi --version
echo "under proj/inside: $(find "$W/proj/inside" -mindepth 1 | wc -l) entries"; echo
run pinfold pi --version

echo "## Cleanup"
run pinfold disk rm --all
rm -rf "$W"
