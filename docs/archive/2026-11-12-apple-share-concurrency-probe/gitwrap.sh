#!/bin/sh
# Guest-side git wrapper for mitigation (c). If .git/logs/HEAD changed since
# this wrapper last ran (a host commit appends to it in place, and in-place
# appends reach the guest at once), wait 1.1 s before running git.
st=/tmp/gitwrap$(pwd | tr / _)
fp() { cksum < .git/logs/HEAD 2>/dev/null; }
if [ "$(fp)" != "$(cat "$st" 2>/dev/null)" ]; then
  echo "gitwrap: host change seen, waiting" >&2
  sleep 1.1
fi
git "$@"; rc=$?
fp > "$st"
exit $rc
