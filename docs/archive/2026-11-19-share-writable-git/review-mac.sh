#!/bin/bash
# Usage: review-mac.sh BIN LABEL. Review fixes 1, 2 and 5 through the real
# binary, private XDG roots and temp projects. LABEL names the binary.
BIN=$1
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d /tmp/pfr.XXXX)
export XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state XDG_CACHE_HOME=$T/cache
export PINFOLD_CPUS=2 PINFOLD_MEMORY=1G PINFOLD_NO_UPDATE_CHECK=1
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=host GIT_AUTHOR_EMAIL=h@x GIT_COMMITTER_NAME=host GIT_COMMITTER_EMAIL=h@x
unset GIT_OPTIONAL_LOCKS
echo "binary: $2; PINFOLD_PROFILE=${PINFOLD_PROFILE-unset}"
run() {
  echo "\$ $*"
  "$@" </dev/null >"$T/out" 2>"$T/err"; rc=$?
  sed 's/^/  out: /' "$T/out"; grep -v image-outdated "$T/err" | sed 's/^/  err: /'; echo "  exit: $rc"
}

printf '\n## Fix 1: SIGTERM during a blocked warning write\n'
P=$T/sig; mkdir "$P"; cd "$P" || exit 1
git init -q -b main; echo a > a; git add a; git commit -qm c0
EDITOR=true "$BIN" config edit >/dev/null 2>&1
"$BIN" start </dev/null >/dev/null 2>&1; echo "start exit: $?"
python3 "$HERE/sigterm.py" "$BIN" "$P"
"$BIN" stop </dev/null >/dev/null 2>&1; echo "stop exit: $?"

printf '\n## Fix 5: warning by GIT_OPTIONAL_LOCKS value\n'
"$BIN" start </dev/null >/dev/null 2>&1
for v in UNSET 1 0 false FALSE no off Off; do
  if [ "$v" = UNSET ]; then "$BIN" -- true </dev/null >/dev/null 2>"$T/err"
  else GIT_OPTIONAL_LOCKS=$v "$BIN" -- true </dev/null >/dev/null 2>"$T/err"; fi
  echo "GIT_OPTIONAL_LOCKS=$v: exit $?, warning $(grep -q git-optional-locks "$T/err" && echo shown || echo silent)"
done
grep git-optional-locks "$T/err" || true
"$BIN" -- true </dev/null 2>&1 >/dev/null | grep git-optional-locks | sed 's/^/  text: /'
"$BIN" stop </dev/null >/dev/null 2>&1

printf '\n## Fix 2: .git appears while a non-Git share owner runs\n'
P=$T/nogit; mkdir "$P"; cd "$P" || exit 1
echo f > f
EDITOR=true "$BIN" config edit >/dev/null 2>&1
run "$BIN" start
ln -s /nonexistent .git
echo "host: .git is a symlink"
run "$BIN" -- true
run "$BIN" start
rm .git; git init -q -b main
echo "host: git init"
run "$BIN" -- true
run "$BIN" start
run "$BIN" stop
run "$BIN" -- sh -c 'echo after-stop'
run "$BIN" stop

printf '\n## cleanup\n'
cd /tmp || exit 1
for d in $("$BIN" disk list 2>/dev/null | awk 'NR > 1 { print $1 }'); do "$BIN" disk rm "$d" 2>&1 | sed 's/^/  /'; done
"$BIN" disk list 2>&1 | sed 's/^/  left: /'
rm -rf "$T"
echo done
