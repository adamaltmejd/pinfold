#!/bin/bash
# The pinned guest kernel's config, for vsock loopback, cgroup and userns.
. ~/proto/lib.sh
mkdata $ST/k.ext4 64M
up k A $ST/k.ext4 >/dev/null
gexec k 'zcat /proc/config.gz | grep -E "VSOCK|VIRTIO_VSOCK|CONFIG_CGROUPS=|CGROUP_PIDS|CONFIG_USER_NS|CONFIG_EXT4_FS=|CONFIG_IKCONFIG"'
kill_vm k
