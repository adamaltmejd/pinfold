# The order of the probe. Run with P set to the binary and T a short
# private directory (its cache may already hold the pinned agents):
#   T=$(mktemp -d /tmp/plp.XXXX) P=… bash driver.sh [CASE…]
# Each step's output goes to logs/<n>-<step>.log, home path scrubbed.
here=$(cd "$(dirname "$0")" && pwd)
export T P
. "$here/env.sh"
scrub() { sed "s#$HOME#/Users/user#g; s#$(id -un)#user#g"; }
cases=${*:-baseline pi-ext pi-ext-predir pi-settings pi-ext-raw pi-models claude-managed claude-managed-raw claude-key-boxwide claude-key-boxwide-raw claude-key-blanked codex all claude-tui-env}
{
  run sw_vers
  run container --version
  run container system status
  run git -C "$here" rev-parse HEAD
  run "$P" artifacts
  echo "== reset: remove every disk and run record of this state root"
  run "$P" disk rm --all
  rm -rf "$XDG_STATE_HOME/pinfold/runs"
  run "$P" disk list
  run "$P" box list --label dev.pinfold.owner
} 2>&1 | scrub >"$LOGS/01-versions-reset.log"
start_fakes
n=10
for c in $cases; do
  n=$((n + 1))
  bash "$here/cases.sh" "$c" 2>&1 | scrub >"$LOGS/$n-$c.log"
done
stop_fakes
{
  run "$P" disk rm --all
  run "$P" disk list
  run "$P" box list --label dev.pinfold.owner
  run container list --all
} 2>&1 | scrub >"$LOGS/99-cleanup.log"
