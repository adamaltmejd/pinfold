# Sourced. One box = one state dir: config.json, vsock/ (0700), jail/,
# console.log, vmm.pid, launcher.pid.
export PATH=$PATH:/usr/sbin:/sbin
FC=$HOME/fc
ST=$HOME/.local/state/fcproto
now() { date +%s%N; }
ms() { echo $(( ($2 - $1) / 1000000 )); }

# mkdata FILE SIZE: an empty box disk owned by the box user, as `up` makes it.
mkdata() { rm -f "$1"; truncate -s "$2" "$1"; chmod 0600 "$1"; mkfs.ext4 -q -F -E root_owner=1000:1000 "$1"; }

# up NAME MODE DATA [--no-jail]: boot; returns when the guest reports ready.
# Prints ready_ms.
up() {
  local name=$1 mode=$2 data=$3 jailflag=${4:-} s k r i d v ro t0 t1 rpid lpid
  s=$ST/$name
  rm -rf "$s"; mkdir -p "$s/jail"; mkdir -m 0700 "$s/vsock"
  if [ -n "$jailflag" ]; then k=$FC/vmlinux r=$FC/rootfs.ext4 i=$FC/init.ext4 d=$data v=$s/vsock
  else k=/vmlinux r=/rootfs.ext4 i=/init.ext4 d=/data.ext4 v=/vsock; fi
  ro=false; [ "$mode" = ro ] && ro=true
  cat >"$s/config.json" <<JSON
{
  "boot-source": { "kernel_image_path": "$k",
    "boot_args": "console=ttyS0 reboot=k panic=1 quiet init=/sbin/pf-guest pf.mode=$mode" },
  "drives": [
    { "drive_id": "rootfs", "path_on_host": "$r", "is_root_device": true, "is_read_only": true },
    { "drive_id": "init", "path_on_host": "$i", "is_root_device": false, "is_read_only": true },
    { "drive_id": "data", "path_on_host": "$d", "is_root_device": false, "is_read_only": $ro }
  ],
  "machine-config": { "vcpu_count": 2, "mem_size_mib": 512 },
  "vsock": { "guest_cid": 3, "uds_path": "$v/v.sock" }
}
JSON
  python3 "$HOME/proto/proxy.py" "$s/vsock/v.sock_3128" "$s/egress.log" >"$s/proxy.err" 2>&1 &
  echo $! >"$s/proxy.pid"
  until [ -S "$s/vsock/v.sock_3128" ]; do :; done
  t0=$(now)
  # One accept on host port 1000; socat unlinks the listener when done, so
  # no host port but 3128 listens once the box is ready.
  socat -u UNIX-LISTEN:"$s/vsock/v.sock_1000" - >"$s/ready" &
  rpid=$!
  until [ -S "$s/vsock/v.sock_1000" ]; do :; done
  "$FC/fcjail" $jailflag "$s" "$FC/firecracker" "$FC/vmlinux" "$FC/rootfs.ext4" "$FC/init.ext4" "$data" \
    >"$s/console.log" 2>"$s/fcjail.err" </dev/null &
  lpid=$!
  echo $lpid >"$s/launcher.pid"
  while kill -0 "$rpid" 2>/dev/null; do
    kill -0 "$lpid" 2>/dev/null || { echo "up $name: launcher died" >&2; cat "$s/fcjail.err" >&2; kill "$rpid"; return 1; }
  done
  t1=$(now)
  echo "ready_ms=$(ms "$t0" "$t1") $(cat "$s/ready")"
}

# gexec NAME CMD: run CMD in the box as the exec service would.
gexec() { printf 'CONNECT 1024\n%s\n' "$2" | socat -,ignoreeof UNIX-CONNECT:"$ST/$1/vsock/v.sock" | sed 1d; }
# gctl NAME REQ: a request to PID 1's control port.
gctl() { printf 'CONNECT 1026\n%s\n' "$2" | socat -,ignoreeof UNIX-CONNECT:"$ST/$1/vsock/v.sock" | sed 1d; }

# kill_vm NAME: SIGKILL the VMM and wait for the launcher and proxy to end.
kill_vm() {
  local s=$ST/$1
  kill -KILL "$(cat "$s/vmm.pid")" 2>/dev/null || true
  while kill -0 "$(cat "$s/launcher.pid")" 2>/dev/null; do :; done
  kill "$(cat "$s/proxy.pid")" 2>/dev/null || true
}
