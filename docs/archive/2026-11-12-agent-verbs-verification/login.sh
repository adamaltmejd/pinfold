# Login routes per agent, and the probes. Uses pinfold.toml as the
# project's .pinfold.toml and fake.py on port 18765. Fixture values only.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/env.sh"
log="$T/fake.jsonl"
: >"$log"
python3 -I "$here/fake.py" 18765 "$log" &
fake=$!
cp "$here/pinfold.toml" .pinfold.toml
"$P" allow
export PFV_CLAUDE_TOKEN=fixture-claude-token PFV_PI_KEY=fixture-pi-key
show() { echo "-- fake saw:"; cat "$log"; : >"$log"; }
record() {
  run=$(ls -t "$XDG_STATE_HOME/pinfold/runs" | head -1)
  "$P" box inspect "$run" | python3 -c 'import json,sys
r=json.load(sys.stdin); l=r["launch"]["record"]
print("-- launch record:", json.dumps({"harnesses":l["harnesses"],"env":l["env"],"routes":l["egress"]["routes"]}))'
}

echo "== claude -p through its login route"
"$P" claude -p hello </dev/null; echo "exit $?"
show; record

echo "== pi -p through its anthropic login route"
"$P" pi -p hello --provider anthropic --model sonnet </dev/null; echo "exit $?"
show; record

echo "== probe 1: claude with ANTHROPIC_API_KEY beside its route variables"
PINFOLD_ENV_ANTHROPIC_API_KEY=fixture-api-key "$P" claude -p hello </dev/null; echo "exit $?"
show
echo "== probe 1, raw: claude's own headers, through a plain route"
PINFOLD_ENV_ANTHROPIC_BASE_URL=http://fake PINFOLD_ENV_ANTHROPIC_API_KEY=fixture-api-key \
  PINFOLD_ENV_CLAUDE_CODE_OAUTH_TOKEN=fixture-oauth-token \
  "$P" claude -p hello </dev/null; echo "exit $?"
show

echo "== probe 2: pi with ANTHROPIC_BASE_URL and no pi login route"
cat >.pinfold.toml <<'EOF'
allow = ["api.github.com"]
[routes]
fake = "127.0.0.1:18765"
EOF
"$P" allow
PINFOLD_ENV_ANTHROPIC_BASE_URL=http://fake PINFOLD_ENV_ANTHROPIC_API_KEY=fixture-api-key \
  "$P" pi -p hello --provider anthropic --model sonnet </dev/null; echo "exit $?"
show
echo "== egress log of that run"
run=$(ls -t "$XDG_STATE_HOME/pinfold/runs" | head -1)
cat "$XDG_STATE_HOME/pinfold/runs/$run/egress.jsonl"

echo "== probe 3: codex's top-level options"
"$P" codex --help </dev/null | grep -nE -- '--sandbox|--dangerously|--full-auto|--ask-for-approval'
"$P" codex --sandbox danger-full-access --version </dev/null; echo "exit $?"

rm .pinfold.toml
"$P" allow
kill "$fake"
