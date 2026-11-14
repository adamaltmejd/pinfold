#!/bin/bash
# Started-box cases for #294, which the build session's sandbox could not
# run. Usage: run-box-cases.sh PATH/TO/pinfold
#
# Needs the local image localhost/pinfold/profile-default:latest; it builds
# and pulls nothing. It uses its own XDG config and state roots, so its
# disks and project state are its own, and removes its disks at the end.
# It uses the real cache root (pinned artifacts and the init), except in
# case C's refusal. It creates boxes only through pinfold, which removes
# each on exit. It uses no credentials. If the container service was
# stopped, it starts it and stops it again at the end.
#
# pinfold's daily maintenance pass prunes dead boxes host-wide and old
# image builds in the shared store when its state root's stamp is missing
# or a day old. The script writes a fresh stamp into its state root, so
# the pass does not run.
#
# Only syntax-checked (bash -n) by the build session: it could not run
# `container` at all.
set -u
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
W=$(mktemp -d "${TMPDIR:-/tmp}/pfbox.XXXX")
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
unset VISUAL EDITOR PINFOLD_PROFILE PINFOLD_ALLOW PINFOLD_ROUTES PINFOLD_CPUS PINFOLD_MEMORY
for v in $(compgen -e | grep '^PINFOLD_ENV_'); do unset "$v"; done
mkdir -p "$W/state/pinfold" && touch "$W/state/pinfold/maintenance"
pinfold() { "$BIN" "$@"; }
run() { echo "\$ $*"; "$@" </dev/null 2>&1; echo "[exit $?]"; echo; }
boxes() { container list --all --format json 2>/dev/null | grep -o '"dev.pinfold.project":"[^"]*"' | sort; }

started=0
if ! container system status >/dev/null 2>&1; then
  container system start --disable-kernel-install && started=1
fi
echo "container service started by this script: $started"
if ! container image inspect localhost/pinfold/profile-default:latest >/dev/null 2>&1; then
  echo "localhost/pinfold/profile-default:latest is missing; build it first. Nothing run."
  [ $started = 1 ] && container system stop
  exit 1
fi

git init -q "$W/proj"
git -C "$W/proj" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
mkdir "$W/plain"
cd "$W/proj"

echo "## A. No configuration: refused, no box"
before=$(boxes)
run pinfold pi --version
[ "$(boxes)" = "$before" ] && echo "box list unchanged"; echo
echo "project state dirs: $(ls "$W/state/pinfold/projects" 2>/dev/null | wc -l | tr -d ' ')"; echo

echo "## B. Configured Git project starts (positive control)"
EDITOR=true run pinfold config edit
run pinfold pi --version
run pinfold push
run pinfold fetch

echo "## C. host-path-in-project: refused before a box; external roots run"
mkdir -p "$W/proj/inside"
ln -s "$W/proj/inside" "$W/cachelink"
before=$(boxes)
# The cache root's suffix under the link does not exist.
XDG_CACHE_HOME=$W/cachelink/missing/cache run pinfold pi --version
[ "$(boxes)" = "$before" ] && echo "box list unchanged"
echo "under proj/inside: $(find "$W/proj/inside" -mindepth 1 | wc -l | tr -d ' ') entries"; echo
run pinfold pi --version

echo "## D. macOS outside Git: scratch, after config edit"
cd "$W/plain"
run pinfold pi --version
EDITOR=true run pinfold config edit
run pinfold pi --version

echo "## Cleanup"
run pinfold disk rm --all
[ $started = 1 ] && container system stop
echo "workdir: $W"
