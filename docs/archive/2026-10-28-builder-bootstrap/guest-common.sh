#!/bin/bash
# Disposable serial-console proof, never installed in the bootstrap.
set -eu
export PATH=/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /run
mkdir -p /dev/pts /dev/shm /sys/fs/cgroup
mount -t devpts devpts /dev/pts
mount -t tmpfs tmpfs /dev/shm
mount -t cgroup2 none /sys/fs/cgroup
mount /dev/vdb /work
mount -o ro /dev/vdc /input
test "$(ls /sys/class/net)" = lo
test "$(cat /proc/sys/user/max_user_namespaces)" = 0
grep -E 'CapBnd|NoNewPrivs' /proc/self/status
test "$(cat /proc/mounts | awk '$2 == "/" {print $4}' | cut -d, -f1)" = ro
buildctl --version
buildkitd --version
buildkit-runc --version
tar --version | head -1
mkfs.ext4 -V
mkfifo /run/buildkit-log /run/ready
while IFS= read -r line; do
	printf '%s\n' "$line"
	case "$line" in
	*'running server'*) echo ready >/run/ready ;;
	esac
done </run/buildkit-log &
logger=$!
setpriv --no-new-privs --bounding-set=-all,+chown,+dac_override,+fowner,+fsetid,+kill,+setgid,+setuid,+setpcap,+net_bind_service,+net_raw,+sys_chroot,+mknod,+audit_write,+setfcap,+sys_admin \
	buildkitd --addr unix:///run/buildkit/buildkitd.sock --root /work/cache \
	--oci-worker-snapshotter=overlayfs --oci-worker-net=host \
	--containerd-worker=false --oci-worker-gc=false >/run/buildkit-log 2>&1 &
daemon=$!
read -r ready </run/ready
test "$ready" = ready
export BUILDKIT_HOST=unix:///run/buildkit/buildkitd.sock
export DOCKER_CONFIG=/tmp/empty-docker
mkdir -p "$DOCKER_CONFIG"
