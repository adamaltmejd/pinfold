# Case 1: each verb starts a box and runs its agent at its pin.
. "$(dirname "$0")/env.sh"
"$P" artifacts | python3 -c 'import json,sys
for a in json.load(sys.stdin): print(a["name"], a["version"], a["cached"])'
for a in pi claude codex; do
  echo "== pinfold $a --version"
  "$P" "$a" --version </dev/null
  echo "exit $?"
done
