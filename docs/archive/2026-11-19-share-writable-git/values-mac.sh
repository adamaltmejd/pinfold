#!/bin/bash
# Usage: values-mac.sh BIN. The warning by GIT_OPTIONAL_LOCKS value, joins of
# a kept share box, private XDG roots and a temp Git project.
BIN=$1
T=$(mktemp -d /tmp/pfr.XXXX)
export XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state XDG_CACHE_HOME=$T/cache
export PINFOLD_CPUS=2 PINFOLD_MEMORY=1G PINFOLD_NO_UPDATE_CHECK=1 GIT_CONFIG_GLOBAL=/dev/null
unset GIT_OPTIONAL_LOCKS
mkdir "$T/p"; cd "$T/p" || exit 1
git init -q -b main; echo a > a; git add a; git -c user.name=h -c user.email=h@x commit -qm c0
EDITOR=true "$BIN" config edit >/dev/null 2>&1
"$BIN" start </dev/null >/dev/null 2>&1
for v in UNSET 1 false 0; do
  if [ "$v" = UNSET ]; then "$BIN" -- true </dev/null >/dev/null 2>"$T/err"
  else GIT_OPTIONAL_LOCKS=$v "$BIN" -- true </dev/null >/dev/null 2>"$T/err"; fi
  echo "GIT_OPTIONAL_LOCKS=$v: exit $?, warning $(grep -q git-optional-locks "$T/err" && echo shown || echo silent)"
done
"$BIN" stop </dev/null >/dev/null 2>&1
cd /tmp || exit 1
for d in $("$BIN" disk list 2>/dev/null | awk 'NR > 1 { print $1 }'); do "$BIN" disk rm "$d"; done
rm -rf "$T"
