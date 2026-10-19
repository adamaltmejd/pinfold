# Sourced by the check scripts on the labvm host.
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
export PATH=/usr/local/bin:$PATH:/usr/sbin:/sbin
PINFOLD=$HOME/pinfold/target/x86_64-unknown-linux-musl/release/pinfold
FCBOX=$HOME/proto/fcbox.sh
OUT=$HOME/proto/out
mkdir -p "$OUT"

# Start a podman box whose proxy also serves the Firecracker guest:
# proxy_up NAME -> sets PROXY_SOCK and EGRESS_LOG.
proxy_up() {
  mkdir -p "$OUT/$1-home"
  spec=$(printf '{"name":"%s","profile":"default","env":{"HOME":"/home/box"},"mounts":[{"host":"%s","guest":"/home/box","readonly":false}],"egress":{"allow":["api.github.com"]}}' "$1" "$OUT/$1-home")
  { printf '%s\n' "$spec"; exec sleep infinity; } | "$PINFOLD" box up >"$OUT/$1.up" 2>"$OUT/$1.err" &
  until grep -q '"ready"' "$OUT/$1.up" 2>/dev/null; do
    kill -0 $! 2>/dev/null || { cat "$OUT/$1.err"; return 1; }
    sleep 0.2
  done
  EGRESS_LOG=$(jq -r .egress_log "$OUT/$1.up")
  key=$(printf '%s' "$1" | sha256sum | cut -c1-16)
  PROXY_SOCK=$HOME/.local/state/pinfold/boxes/$key/proxy.sock
  test -S "$PROXY_SOCK"
}
