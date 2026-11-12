#!/bin/bash
# Part A. Expects P (the binary) and D (this directory) in the environment.
# Each command's stdout, stderr and exit code are printed apart.
export HL_DUMMY_CLAUDE=hl-dummy-claude-value HL_DUMMY_ANTHROPIC=hl-dummy-anthropic-value
run() {
  echo "\$ pinfold $*"
  "$P" "$@" > /tmp/a.out 2> /tmp/a.err < /dev/null
  code=$?
  sed 's/^/  stdout: /' /tmp/a.out
  sed 's/^/  stderr: /' /tmp/a.err
  echo "  exit $code"
}
filter='ANTHROPIC|CLAUDE|PI_|PINFOLD|PROXY|proxy|OPENROUTER|OPENAI|CODEX'
echo "== up"
"$D/up.sh" "$D/three.json" /tmp/three.out
echo "== artifacts"
"$P" artifacts | python3 -c 'import json,sys; [print(a["name"], a["version"]) for a in json.load(sys.stdin)]'
echo "== mounts and versions"
run box exec hl-three -- ls /opt/pinfold /opt/pinfold/login
run box exec hl-three -- sh -c 'grep " /opt/pinfold" /proc/mounts'
for n in pi claude codex; do run box exec hl-three -- /opt/pinfold/$n/$n --version; done
echo "== env per harness"
for h in claude pi codex; do run box exec hl-three --harness $h -- sh -c "env | grep -E '$filter' | sort"; done
run box exec hl-three -- sh -c "env | grep -E '$filter' | sort"
echo "== unlisted harness"
run box exec hl-three --harness nope -- true
run box exec hl-three --harness gemini -- true
echo "== always-applied names"
for h in claude pi codex; do
  run box exec hl-three --harness $h -- sh -c 'echo "$NODE_USE_ENV_PROXY|$HTTPS_PROXY|$http_proxy"'
done
echo "== recorded harness env"
sd=$(dirname "$(find "${XDG_STATE_HOME:-$HOME/.local/state}/pinfold" -name harnesses.json | head -1)")
echo "state dir: $sd"
cat "$sd/harnesses.json"; echo
echo "== dummy values on disk"
grep -rl -e hl-dummy-claude-value -e hl-dummy-anthropic-value "${XDG_STATE_HOME:-$HOME/.local/state}/pinfold" "${XDG_CONFIG_HOME:-$HOME/.config}/pinfold" "${XDG_CACHE_HOME:-$HOME/.cache}/pinfold" 2>/dev/null
echo "grep exit $?"
