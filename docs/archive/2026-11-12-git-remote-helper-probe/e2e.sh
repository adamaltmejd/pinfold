#!/bin/bash
# One fetch from a live box and one from a stopped box's disk through a
# transient box, with the built pinfold binary on Apple's runtime.
# Usage: e2e.sh EMPTY_TEMP_DIR PINFOLD_BINARY. Logs go to ./logs/e2e.log.
# Uses an existing image only (IMAGE below) and an isolated XDG root whose
# daily-pass stamp is fresh, so no image retention or host-wide prune runs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$1
BIN=$2
IMAGE=localhost/pinfold/profile-default:latest
LOG=$HERE/logs/e2e.log
: >"$LOG"

export PATH="$HERE:$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
# No configuration from the calling environment either.
unset GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS $(compgen -e | grep -E '^GIT_CONFIG_(KEY|VALUE)_[0-9]+$')
export GIT_AUTHOR_NAME=probe GIT_AUTHOR_EMAIL=probe@example.invalid
export GIT_COMMITTER_NAME=probe GIT_COMMITTER_EMAIL=probe@example.invalid
export GIT_AUTHOR_DATE='2026-11-12T12:00:00Z' GIT_COMMITTER_DATE='2026-11-12T12:00:00Z'
export XDG_STATE_HOME=$T/xdg/state XDG_CONFIG_HOME=$T/xdg/config XDG_CACHE_HOME=$T/xdg/cache
export PINFOLD_NO_UPDATE_CHECK=1 PROBE_PINFOLD=$BIN PROBE_GUEST_REPO=/work
mkdir -p "$XDG_STATE_HOME/pinfold" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"
touch "$XDG_STATE_HOME/pinfold/maintenance"
H=$T/host
CASE=setup

note() { echo "[$CASE] $*" >>"$LOG"; }
run() {
	echo "[$CASE] \$ $*" >>"$LOG"
	"$@" >>"$LOG" 2>&1
	echo "[$CASE] exit=$?: $*" >>"$LOG"
}
snap() {
	{
		git -C "$H" count-objects -v
		git -C "$H" for-each-ref
		(cd "$H/.git/objects" && find . -type f | sort && find . -type d -name 'pinfold-incoming-*')
		git -C "$H" fsck --no-progress 2>&1
		echo "fsck-exit=$?"
	} >"$T/snap-$1" 2>&1
}
compare() {
	if diff "$T/snap-$1" "$T/snap-$2" >"$T/diff"; then
		note "state: unchanged"
	else
		note "state: CHANGED"
		sed "s/^/[$CASE]   /" "$T/diff" >>"$LOG"
	fi
}
spec() { # name, then the disk's extra fields
	printf '{"name":"%s","image":"%s","disks":[{"name":"grh-probe-work","guest":"/work"%s}]}\n' "$1" "$IMAGE" "$2"
}

run "$BIN" --version
git init -q -b main "$H"
echo host >"$H/h"
git -C "$H" add h
git -C "$H" commit -qm "host one"
git -C "$H" remote add box pinfold::e2e
git clone -q "$H" "$T/seed"
git -C "$T/seed" remote remove origin

CASE=E1-live-box
mkfifo "$T/up-in"
spec grh-probe-live ',"size":"1G","from":"'"$T/seed"'"' >"$T/live.json"
# The owner holds the box while its stdin stays open.
("$BIN" box up <"$T/up-in" >"$T/up-live.out" 2>"$T/up-live.err"; echo "up-exit=$?" >>"$T/up-live.out") &
exec 9>"$T/up-in"
cat "$T/live.json" >&9
for _ in $(seq 1 120); do
	grep -q '"event"' "$T/up-live.out" 2>/dev/null && break
	sleep 1
done
note "up: $(head -1 "$T/up-live.out")"
run "$BIN" box exec grh-probe-live -- sh -c 'cd /work && git config user.name probe && git config user.email probe@example.invalid && echo box >b && git add b && git commit -qm "box one" && git rev-parse HEAD'
export PROBE_BOX=exec:grh-probe-live PROBE_LOG=$HERE/logs/helper-E1-live-box.log
rm -f "$PROBE_LOG"
snap before
run git -C "$H" fetch box
snap after
compare before after
run git -C "$H" for-each-ref
run "$BIN" box exec grh-probe-live -- sh -c 'cd /work && echo box2 >b && git commit -qam "box two" && git rev-parse HEAD'
exec 9>&-
wait
note "up after stdin closed: $(tail -2 "$T/up-live.out" | tr '\n' ' ')"

CASE=E2-stopped-box-transient
run container list --all
spec grh-probe-transient ',"readonly":true' >"$T/transient.json"
export PROBE_BOX=transient:$T/transient.json PROBE_LOG=$HERE/logs/helper-E2-stopped-box-transient.log
rm -f "$PROBE_LOG"
run "$BIN" box exec grh-probe-live -- true
snap before
run git -C "$H" fetch box
snap after
compare before after
run git -C "$H" for-each-ref
run container list --all

CASE=cleanup
run "$BIN" disk rm grh-probe-work
run "$BIN" disk list
