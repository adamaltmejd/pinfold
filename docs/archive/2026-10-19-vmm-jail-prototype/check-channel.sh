#!/bin/bash
# Check 2: the control channel from inside a jailed box (uid 1000).
# Exec takes one line; the probes are one command per port.
. ~/proto/lib.sh
mkdata $ST/c.ext4 256M
up c A $ST/c.ext4
echo "-- host listeners once ready:"; ls $ST/c/vsock
gexec c 'id -u; for p in "2 1024" "2 1026" "2 4444" "2 1000" "2 3128" "1 1024" "1 1026" "3 1024" "3 1026" "1 4444"; do /sbin/pf-guest probe $p; done; echo "vsock modules: $(ls /sys/module | grep -i vsock | tr "\n" " ")"; ls /proc/config.gz 2>&1; echo "loopback symbols: $(grep -c vsock_loopback /proc/kallsyms)"; echo "kallsyms readable: $(head -1 /proc/kallsyms)"'
echo "-- host-initiated connections, as PID 1 logged them:"
gctl c status
grep -a 'ctl:' $ST/c/console.log
kill_vm c
