#!/bin/bash
# Check 1: the rootless VMM jail, observed from the host.
. ~/proto/lib.sh
mkdir -p $ST
echo "== host"
uname -r; id
echo "unprivileged_userns_clone=$(cat /proc/sys/kernel/unprivileged_userns_clone)"
ls /proc/sys/kernel | grep -E 'apparmor|userns' | while read k; do echo "$k=$(sudo cat /proc/sys/kernel/$k)"; done
echo "lsm=$(cat /sys/kernel/security/lsm)"

echo "== boot jailed, proxy over vsock"
mkdata $ST/j.ext4 256M
up j A $ST/j.ext4
L=$(cat $ST/j/launcher.pid) V=$(cat $ST/j/vmm.pid)
cat $ST/j/fcjail.err
echo "launcher=$L vmm=$V"
grep -E '^(Name|PPid|Uid|Gid|Groups|NSpid|CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs|Seccomp|Seccomp_filters)' /proc/$V/status
echo "groups of the launcher: $(grep ^Groups /proc/$L/status)"
for n in user mnt pid net ipc uts cgroup; do
  a=$(readlink /proc/$L/ns/$n) b=$(readlink /proc/$V/ns/$n)
  echo "ns $n launcher=$a vmm=$b $([ "$a" = "$b" ] && echo SAME || echo differs)"
done
echo "-- /proc/$V/mountinfo"
cat /proc/$V/mountinfo
echo "-- mount points (field 5) under /home: $(awk '$5 ~ /^\/home/' /proc/$V/mountinfo | wc -l)"
echo "-- entries naming /home anywhere: $(grep -c /home /proc/$V/mountinfo)"
echo "-- /proc/$V/root listing (as the host sees the VMM's root)"
ls -la /proc/$V/root/ /proc/$V/root/dev /proc/$V/root/vsock 2>&1
echo "-- guest"
echo "host uname -r: $(uname -r)"
gexec j 'uname -r; ip -o link 2>/dev/null || ls /sys/class/net; curl -sS -o /dev/null -w "api.github.com %{http_code}\n" https://api.github.com/; curl -sS -o /dev/null -w "example.com %{http_code}\n" https://example.com/'
echo "-- egress log"; cat $ST/j/egress.log
echo "-- host listeners in the vsock dir once ready"; ls -la $ST/j/vsock
echo "== SIGKILL of the launcher ends the VM"
t0=$(now); kill -KILL $L
until ! kill -0 $V 2>/dev/null; do [ $(ms $t0 $(now)) -gt 5000 ] && { echo "VMM still alive after 5 s"; break; }; done
echo "vmm gone after $(ms $t0 $(now)) ms"
kill $(cat $ST/j/proxy.pid)

echo "== escape probes after every restriction (FCJAIL_PROBE), with and without Landlock"
sleep 1000 & SL=$!
socat ABSTRACT-LISTEN:fcjail-probe-abstract,fork /dev/null & AB=$!
python3 -m http.server --bind 127.0.0.1 8080 >/dev/null 2>&1 & HT=$!
until curl -s -o /dev/null http://127.0.0.1:8080/; do :; done
echo "host controls: tcp 127.0.0.1:8080 $(curl -s -o /dev/null -w %{http_code} http://127.0.0.1:8080/), abstract $(socat -u /dev/null ABSTRACT-CONNECT:fcjail-probe-abstract && echo ok), kill -0 $SL $(kill -0 $SL && echo ok)"
for ll in "" 1; do
  echo "-- FCJAIL_NO_LANDLOCK=${ll:-unset}"
  rm -rf $ST/p; mkdir -p $ST/p/jail; mkdir -m 0700 $ST/p/vsock; cp $ST/j/config.json $ST/p/
  socat UNIX-LISTEN:$ST/p/vsock/v.sock_3128,fork /dev/null & PX=$!; until [ -S $ST/p/vsock/v.sock_3128 ]; do :; done
  FCJAIL_PROBE=1 FCJAIL_NO_LANDLOCK=$ll FCJAIL_PROBE_KILL=$SL $FC/fcjail $ST/p $FC/firecracker $FC/vmlinux $FC/rootfs.ext4 $FC/init.ext4 $ST/j.ext4 2>&1
  kill $PX; ls $ST/p/vsock
done
kill $SL $AB $HT

echo "== unprivileged user namespaces off (Debian knob)"
echo 0 | sudo tee /proc/sys/kernel/unprivileged_userns_clone >/dev/null
rm -rf $ST/p; mkdir -p $ST/p/jail; mkdir -m 0700 $ST/p/vsock; cp $ST/j/config.json $ST/p/
FCJAIL_PROBE=1 $FC/fcjail $ST/p $FC/firecracker $FC/vmlinux $FC/rootfs.ext4 $FC/init.ext4 $ST/j.ext4 2>&1 | head -3; echo "exit=${PIPESTATUS[0]}"
echo 1 | sudo tee /proc/sys/kernel/unprivileged_userns_clone >/dev/null
for k in apparmor_restrict_unprivileged_unconfined unprivileged_userns_apparmor_policy apparmor_restrict_unprivileged_userns; do
  f=/proc/sys/kernel/$k
  [ -e $f ] || { echo "-- $k: absent on this kernel"; continue; }
  old=$(sudo cat $f); echo 1 | sudo tee $f >/dev/null || { echo "-- $k: cannot set"; continue; }
  echo "-- $k=1"
  rm -rf $ST/p; mkdir -p $ST/p/jail; mkdir -m 0700 $ST/p/vsock; cp $ST/j/config.json $ST/p/
  FCJAIL_PROBE=1 $FC/fcjail $ST/p $FC/firecracker $FC/vmlinux $FC/rootfs.ext4 $FC/init.ext4 $ST/j.ext4 2>&1 | grep -E 'fcjail:|KVM_GET' | head -4; echo "exit=${PIPESTATUS[0]}"
  echo $old | sudo tee $f >/dev/null
done
