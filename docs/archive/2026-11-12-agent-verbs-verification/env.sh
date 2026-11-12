# Source with T (a short private directory, e.g. $(mktemp -d /tmp/pfv.XXXX))
# and P (the binary under test) set. The project is $T/${PROJ:-proj}, a
# Git repository with one commit.
export XDG_STATE_HOME="$T/state" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config"
export PINFOLD_NO_UPDATE_CHECK=1
mkdir -p "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
cd "$T/${PROJ:-proj}" || exit 1
