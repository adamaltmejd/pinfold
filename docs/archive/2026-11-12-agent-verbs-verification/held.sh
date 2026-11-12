# Cases 4 and 5: while one agent runs interactively, a second agent's start
# in the same project is refused, and `pinfold attach` gets pi's env.
# HOLD names the agent kept running (default pi). The held agent runs on a
# terminal from pty_run.py, which sends it no input, so it waits.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/env.sh"
HOLD=${HOLD:-pi}
python3 -I "$here/pty_run.py" "$T/out.$HOLD" "$P" "$HOLD" &
# Wait for the box to be ready: attach answers once it is.
i=0
until "$P" attach true </dev/null >/dev/null 2>&1; do
  i=$((i + 1)); [ $i -gt 120 ] && { echo "box never ready"; break; }
  sleep 1
done
for a in pi claude codex; do
  echo "== pinfold $a --version while $HOLD runs"
  "$P" "$a" --version </dev/null
  echo "exit $?"
done
echo "== pinfold attach env (filtered)"
"$P" attach env </dev/null | grep -E '^(ANTHROPIC|CLAUDE|PI_|OPENAI|OPENROUTER|HERDR|HOME=)' | sort
echo "== pinfold attach: the project on the box's disk"
"$P" attach sh -c 'ls -A; git log --oneline -1; git branch --show-current' </dev/null
echo "== end the held agent's launcher with SIGTERM"
launcher=$(pgrep -f "^$P $HOLD\$")
kill -TERM "$launcher"
wait
echo "launcher ended; boxes left:"
"$P" box list --label dev.pinfold.owner
echo "== held agent's stderr lines from pinfold"
tr -d '\r' <"$T/out.$HOLD" | grep '^pinfold' || true
