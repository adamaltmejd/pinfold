# `--help` after an agent verb is the agent's; the launch record lists only
# the launched harness.
. "$(dirname "$0")/env.sh"
"$P" claude --help </dev/null | head -2
"$P" codex --help </dev/null | head -2
echo "== pinfold --help agent lines"
"$P" --help | grep -E '^pinfold (pi|claude|codex|attach) '
echo "== newest launch record"
run=$(ls -t "$XDG_STATE_HOME/pinfold/runs" | head -1)
"$P" box inspect "$run" | python3 -c 'import json,sys
r=json.load(sys.stdin); l=r["launch"]["record"]
print(json.dumps({"harnesses":l["harnesses"],"env":l["env"],"routes":l["egress"]["routes"],"terminal":r["terminal"]["record"]["reason"]}))'
echo "== codex's own Linux sandbox in the box"
"$P" codex sandbox --help </dev/null | sed -n '1,12p'
"$P" codex sandbox linux -- sh -c 'echo inside; touch /tmp/x && echo wrote-tmp' </dev/null; echo "exit $?"
