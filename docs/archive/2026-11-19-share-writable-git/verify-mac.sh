#!/bin/bash
# Usage: verify-mac.sh BIN. The #334 Mac cases through the real binary, with
# private XDG roots and temp Git projects. Prints commands and observations.
BIN=$1
T=$(mktemp -d /tmp/pfv.XXXX)
export XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state XDG_CACHE_HOME=$T/cache
export PINFOLD_CPUS=2 PINFOLD_MEMORY=1G PINFOLD_NO_UPDATE_CHECK=1
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=host GIT_AUTHOR_EMAIL=h@x GIT_COMMITTER_NAME=host GIT_COMMITTER_EMAIL=h@x
unset GIT_OPTIONAL_LOCKS
echo "PINFOLD_PROFILE=${PINFOLD_PROFILE-unset}"

say() { printf '\n## %s\n' "$*"; }
# run CMD...: print it, its stdout, its stderr and its exit code.
run() {
  echo "\$ $*"
  "$@" >"$T/out" 2>"$T/err"; rc=$?
  sed 's/^/  out: /' "$T/out"; sed 's/^/  err: /' "$T/err"; echo "  exit: $rc"
}
warned() { grep -q 'git-optional-locks' "$T/err" && echo "  warning: yes" || echo "  warning: no"; }

new_project() {
  P=$T/$1; mkdir -p "$P"; cd "$P" || exit 1
  git init -q -b main
  seq 1 20 > a; echo b0 > b; git add a b; git commit -qm c0
  EDITOR=true "$BIN" config edit >/dev/null 2>&1
}

say "S1 share: box stages one hunk of two with git add -p and commits"
new_project share
H0=$(git rev-parse HEAD)
run "$BIN" -- sh -c 'sed -i "1s/.*/one/;20s/.*/twenty/" a && printf "y\nn\n" | git add -p a >/dev/null && git commit -qm box1 && git log -1 --format="%s %P"'
warned
echo "host: HEAD subject=$(git log -1 --format=%s) parent_is_H0=$([ "$(git rev-parse HEAD^)" = "$H0" ] && echo yes || echo no)"
echo "host: HEAD:a line1=$(git show HEAD:a | sed -n 1p) line20=$(git show HEAD:a | sed -n 20p); worktree line20=$(sed -n 20p a)"
echo "host: status=$(git status --porcelain | tr '\n' ';') fsck_rc=$(git fsck --no-progress >/dev/null 2>&1; echo $?)"
echo "host: launch record mounts:"
for f in "$XDG_STATE_HOME"/pinfold/runs/*/launch.json; do python3 -c 'import json,sys; [print("  ",m) for m in json.load(open(sys.argv[1]))["mounts"]]' "$f"; done

say "S2 warning with GIT_OPTIONAL_LOCKS=0 and =1"
GIT_OPTIONAL_LOCKS=0 run "$BIN" -- true; warned
GIT_OPTIONAL_LOCKS=1 run "$BIN" -- true; warned

say "S3 host commit, then a box commit 3 s later, 5 times, kept box"
export GIT_OPTIONAL_LOCKS=0
run "$BIN" start
for i in 1 2 3 4 5; do
  echo "h$i" >> b; git commit -qam "host$i"; HC=$(git rev-parse HEAD)
  sleep 3
  "$BIN" -- sh -c "echo g$i >> b && git commit -qam box$i" 2>"$T/err"; rc=$?
  echo "iter $i: box exit=$rc subject=$(git log -1 --format=%s) parent_is_host=$([ "$(git rev-parse HEAD^)" = "$HC" ] && echo yes || echo no) b_lines=$(git show HEAD:b | wc -l | tr -d ' ') warning=$(grep -c git-optional-locks "$T/err")"
done
echo "host: commits on main=$(git rev-list --count main) status=$(git status --porcelain | tr '\n' ';') fsck_rc=$(git fsck --no-progress >/dev/null 2>&1; echo $?)"

say "S4 linked worktree beneath the root commits"
git worktree add -q wt -b side
cd wt || exit 1
WH=$(git rev-parse HEAD)
run "$BIN" -- sh -c 'echo w > w && git add w && git commit -qm wtbox && git log -1 --format=%s'
echo "host: side subject=$(git log -1 --format=%s side) parent_is_start=$([ "$(git rev-parse side^)" = "$WH" ] && echo yes || echo no) main_unchanged=$([ "$(git rev-parse main)" = "$WH" ] && echo yes || echo no)"
cd "$P" || exit 1
run "$BIN" stop
unset GIT_OPTIONAL_LOCKS

say "S5 disk project: no warning, host commit synced, box commit crosses by fetch"
new_project disk
CFG=$(ls "$XDG_CONFIG_HOME"/pinfold/projects/disk-*/pinfold.toml)
sed -i '' 's/kind = "share"/kind = "disk"/' "$CFG"; cat "$CFG" | sed 's/^/  cfg: /'
run "$BIN" -- sh -c 'git log -1 --format=%s && echo d > d && git add d && git commit -qm diskbox'; warned
echo "host: HEAD subject=$(git log -1 --format=%s) (box commit stays in the box)"
run "$BIN" fetch
echo "host: pinfold/main subject=$(git log -1 --format=%s pinfold/main 2>&1)"

say "cleanup"
"$BIN" stop >/dev/null 2>&1
cd /tmp || exit 1
for d in $("$BIN" disk list 2>/dev/null | awk 'NR > 1 { print $1 }'); do "$BIN" disk rm "$d" 2>&1 | sed 's/^/  /'; done
"$BIN" disk list 2>&1 | sed 's/^/  left: /'
rm -rf "$T"
echo done
