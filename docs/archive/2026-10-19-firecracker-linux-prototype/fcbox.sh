#!/bin/sh
# Prototype Firecracker box lifecycle. Not pinfold code.
#   fcbox.sh up NAME PROJECT PROXY_SOCK   boot; prints timings; returns when ready
#   fcbox.sh exec NAME [--workdir D] -- ARGV...
#   fcbox.sh attach NAME
#   fcbox.sh down NAME
# PROXY_SOCK is the unix socket of a running pinfold proxy (here: a podman
# `pinfold box up` with egress).
# FC_JAIL=1 (default): the jailer, which needs root (chroot, mknod, cgroups,
#   pid namespace); Firecracker then runs as the caller's uid in the chroot.
# FC_JAIL=0: rootless. Firecracker runs directly as the caller (who needs
#   /dev/kvm access, the kvm group); no sudo anywhere.
set -eu
fc=$HOME/fc
cmd=$1 name=$2
shift 2
if [ "${FC_JAIL:-1}" = 1 ]; then
  jail=/srv/jailer/firecracker/$name
  root=$jail/root
  p=          # paths as Firecracker sees them, inside the chroot
else
  jail=$HOME/.local/state/fcbox/$name
  root=$jail
  p=$root
fi
now() { date +%s.%N; }
ms() { echo "$1 $2" | awk '{printf "%d", ($2-$1)*1000}'; }

case $cmd in
up)
  project=$1 proxy=$2
  t0=$(now)
  if [ "${FC_JAIL:-1}" = 1 ]; then
    sudo mkdir -p "$root"
    sudo chown -R "$(id -u):$(id -g)" "$jail"
  else
    mkdir -p -m 0700 "$root"
  fi
  ln "$fc/vmlinux" "$root/vmlinux"
  ln "$fc/rootfs.ext4" "$root/rootfs.ext4"
  # Copy-in: the project tree, .git included, becomes the box's own disk.
  # Size: the tree plus 50% and 256 MiB of headroom.
  kib=$(du -sk "$project" | cut -f1)
  size=$(( kib * 3 / 2 + 262144 ))K
  mkfs.ext4 -q -F -E root_owner=1000:1000 -d "$project" "$root/work.ext4" "$size"
  t1=$(now)
  # Egress: the guest's CID 2 port 3128 is Firecracker's ${uds}_3128, which
  # must be the proxy socket itself. In the chroot that takes a bind mount;
  # rootless, a symlink (pinfold would simply name its socket so).
  if [ "${FC_JAIL:-1}" = 1 ]; then
    : >"$root/v.sock_3128"
    sudo mount --bind "$proxy" "$root/v.sock_3128"
  else
    ln -s "$proxy" "$root/v.sock_3128"
  fi
  cat >"$root/config.json" <<JSON
{
  "boot-source": {
    "kernel_image_path": "$p/vmlinux",
    "boot_args": "console=ttyS0 reboot=k panic=1 quiet ro init=/sbin/pf-init"
  },
  "drives": [
    { "drive_id": "rootfs", "path_on_host": "$p/rootfs.ext4", "is_root_device": true, "is_read_only": true },
    { "drive_id": "work", "path_on_host": "$p/work.ext4", "is_root_device": false, "is_read_only": false }
  ],
  "machine-config": { "vcpu_count": ${FC_VCPUS:-2}, "mem_size_mib": ${FC_MEM:-1024} },
  "vsock": { "guest_cid": 3, "uds_path": "$p/v.sock" }
}
JSON
  "$fc/pf-agent" wait-ready "$root/v.sock_1000" >"$jail/ready" &
  ready=$!
  if [ "${FC_JAIL:-1}" = 1 ]; then
    sudo "$fc/jailer" --id "$name" --exec-file "$fc/firecracker" \
      --uid "$(id -u)" --gid "$(id -g)" --chroot-base-dir /srv/jailer \
      --cgroup-version 2 --new-pid-ns -- \
      --config-file /config.json --no-api >"$jail/console.log" 2>&1 </dev/null &
  else
    setsid "$fc/firecracker" --config-file "$root/config.json" --no-api \
      >"$jail/console.log" 2>&1 </dev/null &
    echo $! >"$root/firecracker.pid"
  fi
  wait "$ready"
  t2=$(now)
  echo "copy_in_ms=$(ms "$t0" "$t1") boot_ms=$(ms "$t1" "$t2") $(cat "$jail/ready")"
  ;;
exec)
  exec "$fc/pf-agent" exec "$root/v.sock" "$@"
  ;;
attach)
  old=$(stty -g)
  stty raw -echo
  "$fc/pf-agent" attach "$root/v.sock" || true
  stty "$old"
  ;;
down)
  if [ "${FC_JAIL:-1}" = 1 ]; then
    pid=$(sudo cat "$root/firecracker.pid" 2>/dev/null || true)
    [ -n "$pid" ] && sudo kill -KILL "$pid" 2>/dev/null || true
    while [ -n "$pid" ] && sudo kill -0 "$pid" 2>/dev/null; do :; done
    sudo umount "$root/v.sock_3128" 2>/dev/null || true
    sudo rm -rf "$jail"
    sudo rmdir "/sys/fs/cgroup/firecracker/$name" 2>/dev/null || true
  else
    pid=$(cat "$root/firecracker.pid" 2>/dev/null || true)
    [ -n "$pid" ] && kill -KILL "$pid" 2>/dev/null || true
    while [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; do :; done
    rm -rf "$jail"
  fi
  ;;
*) echo "usage: fcbox.sh up|exec|attach|down NAME ..." >&2; exit 2 ;;
esac
