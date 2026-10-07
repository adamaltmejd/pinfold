#!/usr/bin/env bash
# Guarantee 36. Only the fixture namespace changes DNS and host routing.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "$0")" && pwd -P)

if [[ ${1:-} == --namespace ]]; then
  fixture_user=$2
  fixture_root=$3
  binary=$4
  image=$5
  [[ $(id -u) == 0 ]]
  if [[ $(readlink /proc/self/ns/net) == $(readlink /proc/1/ns/net) ||
  $(readlink /proc/self/ns/mnt) == $(readlink /proc/1/ns/mnt) ]]; then
    echo "fixture setup requires separate network and mount namespaces" >&2
    exit 1
  fi
  mount --make-rprivate /
  ip link set lo up
  ip address add 8.8.8.8/32 dev lo
  mount --bind "$fixture_root/hosts" /etc/hosts
  # A namespace-local listener on CONNECT's required port, without giving
  # the unprivileged fixture a capability or changing the global sysctl.
  sysctl -q -w net.ipv4.ip_unprivileged_port_start=0
  exec runuser -u "$fixture_user" -- python3 \
    "$fixture_root/guest/connect_tunnels_share_activity.py" \
    host "$binary" "$fixture_root" "$image"
fi

if [[ $# != 1 || $(uname -s) != Linux || $(id -u) == 0 ]]; then
  echo "usage: scripts/slow-proxy.sh /absolute/path/to/pinfold (unprivileged Linux user)" >&2
  exit 1
fi
binary=$(realpath -- "$1")
[[ -x $binary ]]
fixture_user=$(id -un)
fixture_root=$(mktemp -d /tmp/pinfold-connect.XXXXXXXX)
image_name="connect-proof-$$"
export XDG_STATE_HOME="$fixture_root/state"
export XDG_CACHE_HOME="$fixture_root/cache"
export XDG_CONFIG_HOME="$fixture_root/config"

cleanup() {
  "$binary" image rm "$image_name" || true
  rm -rf -- "$fixture_root"
}
trap cleanup EXIT

umask 077
mkdir -p "$fixture_root/guest"
cp "$script_dir/connect_tunnels_share_activity.py" "$fixture_root/guest/"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj '/CN=Pinfold CONNECT fixture CA' \
  -addext 'basicConstraints=critical,CA:TRUE,pathlen:0' \
  -addext 'keyUsage=critical,keyCertSign,cRLSign' \
  -keyout "$fixture_root/ca.key" -out "$fixture_root/guest/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj '/CN=api.github.com' \
  -keyout "$fixture_root/server.key" -out "$fixture_root/server.csr" 2>/dev/null
cat >"$fixture_root/server.ext" <<'EXT'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:api.github.com
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid,issuer
EXT
openssl x509 -req -days 1 -in "$fixture_root/server.csr" \
  -CA "$fixture_root/guest/ca.pem" -CAkey "$fixture_root/ca.key" -CAcreateserial \
  -extfile "$fixture_root/server.ext" -out "$fixture_root/server.pem" 2>/dev/null
python3 - "$fixture_root/hosts" <<'PY'
from pathlib import Path
import sys

lines = []
for line in Path("/etc/hosts").read_text().splitlines():
    record, separator, comment = line.partition("#")
    fields = record.split()
    if "api.github.com" in fields[1:]:
        fields = [field for field in fields if field != "api.github.com"]
        line = " ".join(fields) + (separator + comment if separator else "") if len(fields) > 1 else ""
    lines.append(line)
Path(sys.argv[1]).write_text("\n".join(lines) + "\n8.8.8.8 api.github.com\n")
PY

# Build before entering the namespace, which has no external network route.
"$binary" build --profile default
image=$(
  "$binary" image build "$image_name" \
    --containerfile "$script_dir/connect-tls.Containerfile" --context "$script_dir" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["ref"])'
)
sudo --preserve-env=PATH,XDG_RUNTIME_DIR,DBUS_SESSION_BUS_ADDRESS \
  unshare --net --mount --fork --kill-child \
  bash "$script_dir/slow-proxy.sh" --namespace \
  "$fixture_user" "$fixture_root" "$binary" "$image"
