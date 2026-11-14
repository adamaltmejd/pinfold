#!/bin/bash
# Started-box cases for #294, which the build session's sandbox could not
# run. Usage: run-box-cases.sh PATH/TO/pinfold
#
# Needs the local image localhost/pinfold/profile-default:latest; it builds
# and pulls nothing. It uses its own XDG config and state roots, so its
# disks and project state are its own, and removes its disks at the end.
# It creates boxes only through pinfold, which removes each on exit. It
# uses no credentials. If the container service was stopped, it starts it
# and stops it again at the end.
set -u
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
W=$(mktemp -d "${TMPDIR:-/tmp}/pfbox.XXXX")
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
unset VISUAL EDITOR PINFOLD_PROFILE PINFOLD_ALLOW PINFOLD_ROUTES
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
run "$BIN" pi --version
[ "$(boxes)" = "$before" ] && echo "box list unchanged"; echo
echo "project state dirs: $(ls "$W/state/pinfold/projects" 2>/dev/null | wc -l)"; echo

echo "## B. Configured Git project starts (positive control)"
EDITOR=true run "$BIN" config edit
run "$BIN" pi --version
run "$BIN" push
run "$BIN" fetch

echo "## C. host-path-in-project: refused before a box; external roots run"
mkdir -p "$W/proj/inside"
ln -s "$W/proj/inside" "$W/statelink"
mkdir -p "$W/config2/pinfold"; cp -R "$W/config/pinfold/projects" "$W/config2/pinfold/"
before=$(boxes)
XDG_STATE_HOME=$W/statelink/missing/state XDG_CONFIG_HOME=$W/config2 run "$BIN" pi --version
[ "$(boxes)" = "$before" ] && echo "box list unchanged"; echo
run "$BIN" pi --version

echo "## D. macOS outside Git: scratch, after config edit"
cd "$W/plain"
run "$BIN" pi --version
EDITOR=true run "$BIN" config edit
run "$BIN" pi --version

echo "## Cleanup"
run "$BIN" disk rm --all
[ $started = 1 ] && container system stop
echo "workdir: $W"
