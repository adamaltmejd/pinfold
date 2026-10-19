#!/bin/bash
# Startup to first exec, copy-in and memory: Firecracker vs podman `box up`
# on the same host. Usage: measure.sh PROXY_SOCK
set -eu
cd "$(dirname "$0")"
. ./env.sh
PROXY_SOCK=$1
fx=$HOME/proto/fixture
now() { date +%s%N; }
drop() { sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null; }

pod_once() {
  spec=$(printf '{"name":"m-pod","image":"pinfold/profile-default:latest","mounts":[{"host":"%s","guest":"/workspace","readonly":false}],"egress":{"allow":["api.github.com"]}}' "$fx")
  t0=$(now)
  { printf '%s\n' "$spec"; exec sleep infinity; } | "$PINFOLD" box up >"$OUT/m-pod.up" 2>&1 &
  until grep -q '"ready"' "$OUT/m-pod.up"; do :; done
  "$PINFOLD" box exec m-pod -- true
  t1=$(now)
  mem=$("$PINFOLD" box stat m-pod | jq -c .memory)
  echo "podman $1 first_exec_ms=$(( (t1 - t0) / 1000000 )) memory=$mem"
  "$PINFOLD" box down m-pod >/dev/null
  pkill -x -P $$ sleep; wait
}

fc_once() {
  t0=$(now)
  "$FCBOX" up m-fc "$fx" "$PROXY_SOCK" >"$OUT/m-fc.up"
  "$FCBOX" exec m-fc -- true
  t1=$(now)
  if [ "${FC_JAIL:-1}" = 1 ]; then
    pid=$(sudo cat /srv/jailer/firecracker/m-fc/root/firecracker.pid)
  else
    pid=$(cat "$HOME/.local/state/fcbox/m-fc/firecracker.pid")
  fi
  rss=$(ps -o rss= -p "$pid" | tr -d ' ')
  used=$("$FCBOX" exec m-fc -- free -k | awk '/Mem:/{print $3}')
  echo "firecracker jail=${FC_JAIL:-1} $1 first_exec_ms=$(( (t1 - t0) / 1000000 )) $(cat "$OUT/m-fc.up") vmm_rss_kib=$rss guest_used_kib=$used"
  "$FCBOX" down m-fc
}

drop; pod_once cold
for i in 1 2 3 4 5; do pod_once warm; done
for FC_JAIL in 1 0; do
  export FC_JAIL
  drop; fc_once cold
  for i in 1 2 3 4 5; do fc_once warm; done
done

# Copy-in: mkfs.ext4 -d of the tree, cold cache, three sizes.
for tree in "$fx" "$HOME/proto/pinfold-git" "$HOME/pinfold"; do
  drop
  kib=$(du -sk "$tree" | cut -f1)
  files=$(find "$tree" | wc -l)
  t0=$(now)
  mkfs.ext4 -q -F -E root_owner=1000:1000 -d "$tree" /tmp/copyin.ext4 "$(( kib * 3 / 2 + 262144 ))K"
  t1=$(now)
  echo "copy-in tree=$tree kib=$kib files=$files ms=$(( (t1 - t0) / 1000000 ))"
  rm -f /tmp/copyin.ext4
done
