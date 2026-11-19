#!/bin/bash
# Usage: verify-linux.sh BIN. The #334 Linux cases: a disk project still runs
# and crosses by fetch, and shares are still refused. Private XDG roots.
BIN=$1
T=$(mktemp -d /tmp/pfl.XXXX)
export XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state XDG_CACHE_HOME=$T/cache
export PINFOLD_CPUS=2 PINFOLD_MEMORY=1G PINFOLD_NO_UPDATE_CHECK=1 PINFOLD_PROFILE=pfl
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=host GIT_AUTHOR_EMAIL=h@x GIT_COMMITTER_NAME=host GIT_COMMITTER_EMAIL=h@x
unset GIT_OPTIONAL_LOCKS

say() { printf '\n## %s\n' "$*"; }
run() {
  echo "\$ $*"
  "$@" >"$T/out" 2>"$T/err" <"${STDIN:-/dev/null}"; rc=$?
  sed 's/^/  out: /' "$T/out"; sed 's/^/  err: /' "$T/err"; echo "  exit: $rc"
}
warned() { grep -q 'git-optional-locks' "$T/err" && echo "  warning: yes" || echo "  warning: no"; }

say "profile pfl: debian:trixie-slim plus git"
"$BIN" profile new pfl --from default >/dev/null
printf 'FROM debian:trixie-slim\nRUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*\nRUN git config --system user.name pi && git config --system user.email pi@pinfold.invalid\n' \
  > "$XDG_CONFIG_HOME/pinfold/profiles/pfl/Containerfile"
P=$T/proj; mkdir -p "$P"; cd "$P" || exit 1
git init -q -b main; echo a > a; git add a; git commit -qm c0
EDITOR=true "$BIN" config edit >/dev/null 2>&1
CFG=$(ls "$XDG_CONFIG_HOME"/pinfold/projects/*/pinfold.toml); sed 's/^/  cfg: /' "$CFG"
"$BIN" build >"$T/build.log" 2>&1; echo "build exit: $?"

say "L1 disk project runs, no warning, box commit crosses by fetch"
run "$BIN" -- sh -c 'git log -1 --format=%s && echo d > d && git add d && git commit -qm lbox'; warned
run "$BIN" fetch
echo "host: pinfold/main subject=$(git log -1 --format=%s pinfold/main 2>&1)"

say "L2 project share configuration refused"
sed -i 's/kind = "disk"/kind = "share"/' "$CFG"; sed 's/^/  cfg: /' "$CFG"
run "$BIN" -- true
sed -i 's/kind = "share"/kind = "disk"/' "$CFG"

say "L3 core box up with a share refused"
IMG=$("$BIN" config --json 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["image"])')
mkdir -p "$T/shared"
printf '{"name":"pfl334","image":"%s","mounts":[{"host":"%s","guest":"/work","kind":"share"}]}\n' "$IMG" "$T/shared" > "$T/spec.json"
STDIN=$T/spec.json run "$BIN" box up

say "cleanup"
"$BIN" stop >/dev/null 2>&1
cd /tmp || exit 1
for d in $("$BIN" disk list 2>/dev/null | awk 'NR > 1 { print $1 }'); do "$BIN" disk rm "$d" 2>&1 | sed 's/^/  /'; done
"$BIN" disk list 2>&1 | sed 's/^/  left: /'
rm -rf "$T"
echo done
