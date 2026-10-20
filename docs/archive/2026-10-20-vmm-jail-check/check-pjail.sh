#!/bin/bash
# #139: pinfold's jail module (crates/pinfold/src/core/runtime/firecracker/jail.rs),
# driven by the scratch `pjail` in place of `box up`, observed from the host
# as row 43 describes. Reuses the 2026-10-19 prototype's guest and proxy.
. ~/proto/lib.sh
. ~/.cargo/env
set -u
mkdir -p $ST
PJ=~/pjail/target/x86_64-unknown-linux-musl/release/pjail

# pjup NAME DATA [FIRECRACKER]: as lib.sh's `up`, with pinfold's jail and its paths.
pjup() {
  local name=$1 data=$2 fc=${3:-$FC/firecracker} s rpid lpid
  s=$ST/$name
  rm -rf "$s"; mkdir -p "$s/jail"; mkdir -m 0700 "$s/vsock"
  cat >"$s/config.json" <<JSON
{
  "boot-source": { "kernel_image_path": "/vmlinux",
    "boot_args": "console=ttyS0 reboot=k panic=1 quiet init=/sbin/pf-guest pf.mode=A" },
  "drives": [
    { "drive_id": "rootfs", "path_on_host": "/root.ext4", "is_root_device": true, "is_read_only": true },
    { "drive_id": "init", "path_on_host": "/init.ext4", "is_root_device": false, "is_read_only": true },
    { "drive_id": "data", "path_on_host": "/disk0.ext4", "is_root_device": false, "is_read_only": false }
  ],
  "machine-config": { "vcpu_count": 1, "mem_size_mib": 256 },
  "vsock": { "guest_cid": 3, "uds_path": "/vsock/v.sock" }
}
JSON
  python3 "$HOME/proto/proxy.py" "$s/vsock/v.sock_3128" "$s/egress.log" >"$s/proxy.err" 2>&1 &
  echo $! >"$s/proxy.pid"
  until [ -S "$s/vsock/v.sock_3128" ]; do :; done
  if [ "$fc" != "$FC/firecracker" ]; then
    socat -u UNIX-LISTEN:"$s/vsock/v.sock_9" - >"$s/probe.out" &
    until [ -S "$s/vsock/v.sock_9" ]; do :; done
    "$PJ" "$s" "$fc" "$FC/vmlinux" "$FC/rootfs.ext4" "$FC/init.ext4" "$data" 2>&1
    echo "pjail exit=$?"
    wait $!; cat "$s/probe.out"
    kill "$(cat "$s/proxy.pid")"
    return
  fi
  socat -u UNIX-LISTEN:"$s/vsock/v.sock_1000" - >"$s/ready" &
  rpid=$!
  until [ -S "$s/vsock/v.sock_1000" ]; do :; done
  "$PJ" "$s" "$fc" "$FC/vmlinux" "$FC/rootfs.ext4" "$FC/init.ext4" "$data" \
    >"$s/console.log" 2>"$s/pjail.err" </dev/null &
  lpid=$!
  echo $lpid >"$s/launcher.pid"
  while kill -0 "$rpid" 2>/dev/null; do
    kill -0 "$lpid" 2>/dev/null || { echo "up $name: pjail died"; cat "$s/pjail.err"; kill "$rpid"; return 1; }
  done
  echo "$name ready: $(cat "$s/ready")"
}

echo "== host"; uname -r; id
echo "landlock abi: $(python3 -c 'import ctypes;print(ctypes.CDLL(None).syscall(444,None,0,1))')"

echo "== two boxes up"
mkdata $ST/a.ext4 128M; mkdata $ST/b.ext4 128M
# A descriptor without close-on-exec, as a careless caller would pass to
# `box up`. pjail holds it; the VMM must not.
exec 7</etc/hostname
pjup a $ST/a.ext4 || exit 1
pjup b $ST/b.ext4 || exit 1
for n in a b; do
  L=$(cat $ST/$n/launcher.pid) V=$(cat $ST/$n/vmm.pid)
  echo "-- box $n: pjail=$L vmm=$V"
  grep -E '^(Name|PPid|Uid|Gid|Groups|NSpid|CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs|Seccomp)' /proc/$V/status
  for ns in user mnt pid net ipc uts cgroup; do
    x=$(readlink /proc/$L/ns/$ns) y=$(readlink /proc/$V/ns/$ns)
    echo "ns $ns $([ "$x" = "$y" ] && echo SAME || echo differs)"
  done
  echo "root: $(ls -A /proc/$V/root/ | tr '\n' ' ') | dev: $(ls /proc/$V/root/dev) | vsock: $(ls /proc/$V/root/vsock | tr '\n' ' ')"
  echo "disk0 inode $(stat -Lc %i /proc/$V/root/disk0.ext4); a.ext4 $(stat -c %i $ST/a.ext4); b.ext4 $(stat -c %i $ST/b.ext4)"
  echo "pjail fd 7: $(readlink /proc/$L/fd/7)"
  echo "vmm fds: $(for f in /proc/$V/fd/*; do printf '%s=%s ' "${f##*/}" "$(readlink "$f")"; done)"
  echo "mount points: $(awk '{print $5}' /proc/$V/mountinfo | tr '\n' ' ')"
  echo "mount points under $HOME: $(awk -v h="$HOME" 'index($5, h) == 1' /proc/$V/mountinfo | wc -l)"
  gexec $n 'curl -sS -o /dev/null -w "api.github.com %{http_code}\n" https://api.github.com/; curl -sS -o /dev/null -w "example.com %{http_code}\n" https://example.com/'
done
exec 7<&-
echo "== SIGKILL of a's pjail ends a's VM; b stays"
La=$(cat $ST/a/launcher.pid) Va=$(cat $ST/a/vmm.pid) Vb=$(cat $ST/b/vmm.pid)
t0=$(now); kill -KILL $La
until ! kill -0 $Va 2>/dev/null; do [ $(ms $t0 $(now)) -gt 5000 ] && { echo "VMM a still alive after 5 s"; break; }; done
echo "vmm a gone after $(ms $t0 $(now)) ms; vmm b alive: $(kill -0 $Vb && echo yes)"
kill -KILL $(cat $ST/b/launcher.pid); until ! kill -0 $Vb 2>/dev/null; do :; done
kill $(cat $ST/a/proxy.pid) $(cat $ST/b/proxy.pid) 2>/dev/null

echo "== escape probes as /firecracker inside the jail"
sleep 1000 & SL=$!
socat ABSTRACT-LISTEN:fcjail-probe-abstract,fork /dev/null & AB=$!
python3 -m http.server --bind 127.0.0.1 8080 >/dev/null 2>&1 & HT=$!
until curl -s -o /dev/null http://127.0.0.1:8080/; do :; done
echo "host controls: tcp $(curl -s -o /dev/null -w %{http_code} http://127.0.0.1:8080/), abstract $(socat -u /dev/null ABSTRACT-CONNECT:fcjail-probe-abstract && echo ok), kill -0 $(kill -0 $SL && echo ok)"
PJ_KILL=$SL pjup p $ST/a.ext4 $FC/pjprobe
echo "vsock dir after probes: $(ls $ST/p/vsock | tr '\n' ' ')"
kill $SL $AB $HT

echo "== preflight's user-namespace probe"
$PJ probe
echo 0 | sudo tee /proc/sys/kernel/unprivileged_userns_clone >/dev/null
$PJ probe
echo 1 | sudo tee /proc/sys/kernel/unprivileged_userns_clone >/dev/null

echo "== a failing step is named"
rm -rf $ST/f; mkdir -p $ST/f/jail; mkdir -m 0700 $ST/f/vsock; echo '{}' > $ST/f/config.json
$PJ $ST/f $FC/firecracker $FC/vmlinux $FC/missing.ext4 $FC/init.ext4 $ST/a.ext4; echo "exit=$?"
