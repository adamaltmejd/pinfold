# Case 3: `pinfold claude` on a fresh project gets the start sync.
# Run with PROJ naming a project no agent has started.
. "$(dirname "$0")/env.sh"
"$P" disk list
echo "== pinfold claude --version"
"$P" claude --version </dev/null
echo "exit $?"
echo "== pinfold fetch"
"$P" fetch
echo "exit $?"
echo "host HEAD:      $(git rev-parse HEAD)"
echo "pinfold/trunk:  $(git rev-parse refs/remotes/pinfold/trunk)"
