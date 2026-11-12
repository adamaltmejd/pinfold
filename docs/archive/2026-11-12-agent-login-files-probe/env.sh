# Source with T (a short private directory, e.g. $(mktemp -d /tmp/plp.XXXX))
# and P (the binary under test) set. XDG roots point into T. Fixture values
# are made up; the box only ever sees placeholders.
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config"
export PINFOLD_NO_UPDATE_CHECK=1
export FAKE_CLAUDE_TOKEN=FAKE-claude-token FAKE_PI_KEY=FAKE-pi-key
export FAKE_CODEX_TOKEN=FAKE-codex-token FAKE_CODEX_ACCOUNT=FAKE-codex-account
mkdir -p "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$T/files"
LOGS="$here/logs"

# Print the command, run it, print its exit code. Output goes to stdout,
# which the driver tees into a log.
run() {
  echo "\$ $*"
  "$@"
  echo "[exit $?]"
}

# One fake per route, so a request's port says which route carried it.
# claude 18771, pi 18772, codex 18773, raw 18774 (a plain host route).
start_fakes() {
  for port in 18771 18772 18773 18774; do
    : >"$T/fake-$port.jsonl"
    python3 -I "$here/fake.py" "$port" "$T/fake-$port.jsonl" &
    echo $! >>"$T/fakes.pid"
  done
}
stop_fakes() { kill $(cat "$T/fakes.pid") 2>/dev/null; rm -f "$T/fakes.pid"; }

# Print and clear what each fake saw.
show_fakes() {
  for port in 18771 18772 18773 18774; do
    if [ -s "$T/fake-$port.jsonl" ]; then
      echo "-- fake $port saw:"
      cat "$T/fake-$port.jsonl"
      : >"$T/fake-$port.jsonl"
    fi
  done
}

# Start box $1 from spec file $2 with stdin held open; wait for its ready
# line. Its owner's stdout goes to $T/up-$1.log.
box_up() {
  local name=$1 spec=$2 fifo="$T/stdin-$1"
  rm -f "$fifo"; mkfifo "$fifo"
  "$P" box up <"$fifo" >"$T/up-$name.log" 2>&1 &
  echo $! >"$T/up-$name.pid"
  exec 9>"$fifo"
  cat "$spec" >&9
  for _ in $(seq 600); do
    grep -q '"event":"ready"' "$T/up-$name.log" && break
    grep -q '"event":"\(failed\|refused\|down\)"' "$T/up-$name.log" && break
    sleep 0.5
  done
  echo "-- up $name:"; cat "$T/up-$name.log"
}

# Tear box $1 down and wait for its owner.
box_down() {
  run "$P" box down "$1"
  exec 9>&-
  wait "$(cat "$T/up-$1.pid")" 2>/dev/null
  echo "-- up $1 ended:"; tail -1 "$T/up-$1.log"
}

# The newest run's egress log.
egress() {
  local run
  run=$(ls -t "$XDG_STATE_HOME/pinfold/runs" | head -1)
  echo "-- egress log of run $run:"
  cat "$XDG_STATE_HOME/pinfold/runs/$run/egress.jsonl" 2>/dev/null
}

x() { "$P" box exec "$BOX" -- "$@"; }
