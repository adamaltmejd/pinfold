# A login route naming no agent is still refused; another agent's route is
# left out. Fixture values only.
. "$(dirname "$0")/env.sh"
export PFV_KEY=fixture-key
printf '[routes]\nx = { login = "nope", from = "PFV_KEY" }\n' >.pinfold.toml
"$P" allow
echo "== login = \"nope\""
"$P" claude --version </dev/null; echo "exit $?"
printf '[routes]\nx = { login = "pi", provider = "openrouter", from = "PFV_KEY" }\n' >.pinfold.toml
"$P" allow
echo "== a pi route, pinfold claude"
"$P" claude --version </dev/null; echo "exit $?"
echo "== the same, pinfold pi"
"$P" pi --version </dev/null; echo "exit $?"
rm .pinfold.toml
"$P" allow
"$P" disk rm --all
