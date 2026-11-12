# Remove every disk this state root created, and show nothing is left.
. "$(dirname "$0")/env.sh"
"$P" disk rm --all
"$P" disk list
"$P" box list --label dev.pinfold.owner
