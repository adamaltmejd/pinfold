#!/bin/sh
# Runs inside the L1 box (Apple container, --virtualization, custom kernel).
# Checks /dev/kvm, then boots a Firecracker arm64 guest and runs one command
# over vsock. Writes results to /work.
set -u
export DEBIAN_FRONTEND=noninteractive
now() { date +%s.%N; }
ms() { echo "$1 $2" | awk '{printf "%d", ($2-$1)*1000}'; }
echo "== uname"; uname -a
echo "== boot EL and kvm dmesg"; dmesg | grep -iE 'started at EL|kvm|vgic|gic' 
echo "== /dev/kvm"; ls -l /dev/kvm
set -e
apt-get update -qq
apt-get install -y -qq --no-install-recommends curl ca-certificates socat e2fsprogs python3 >/dev/null
# KVM_GET_API_VERSION = _IO(0xAE, 0); KVM_CHECK_EXTENSION = _IO(0xAE, 3) with
# KVM_CAP_ARM_VM_IPA_SIZE (165); KVM_CREATE_VM = _IO(0xAE, 1).
python3 -c 'import os,fcntl; fd=os.open("/dev/kvm",os.O_RDWR); print("KVM_GET_API_VERSION", fcntl.ioctl(fd,0xAE00)); print("KVM_CAP_ARM_VM_IPA_SIZE", fcntl.ioctl(fd,0xAE03,165)); vm=fcntl.ioctl(fd,0xAE01,0); print("KVM_CREATE_VM fd", vm)'
fc=v1.17.0
kernel=firecracker-ci/20261016-5732b8331e1e-0/aarch64/vmlinux-6.1.188
mkdir -p /fc/dl && cd /fc/dl
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-aarch64.tgz"
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-aarch64.tgz.sha256.txt"
sha256sum -c "firecracker-$fc-aarch64.tgz.sha256.txt"
tar -xzf "firecracker-$fc-aarch64.tgz"
install -m 0755 "release-$fc-aarch64/firecracker-$fc-aarch64" /fc/firecracker
curl -fsSL -o /fc/vmlinux "https://s3.amazonaws.com/spec.ccfc.min/$kernel"
cd /fc
sha256sum firecracker vmlinux
./firecracker --version | head -1
# Guest rootfs: this box's own Debian userland, with socat, as a read-only
# ext4. Its init listens on vsock port 1000 and runs a shell per connection.
mkdir -p /fc/rootfs
tar -C / -cf - --one-file-system --exclude=./fc --exclude=./work --exclude=./proc --exclude=./sys --exclude=./dev . | tar -C /fc/rootfs -xf -
mkdir -p /fc/rootfs/proc /fc/rootfs/sys /fc/rootfs/dev
cat >/fc/rootfs/sbin/pf-init <<'INIT'
#!/bin/sh
mount -t proc proc /proc; mount -t sysfs sys /sys; mount -t devtmpfs dev /dev
exec socat VSOCK-LISTEN:1000,fork EXEC:/bin/sh,stderr
INIT
chmod 0755 /fc/rootfs/sbin/pf-init
mkfs.ext4 -q -F -d /fc/rootfs /fc/rootfs.ext4 1G
rm -rf /fc/rootfs
cat >/fc/config.json <<JSON
{
  "boot-source": {
    "kernel_image_path": "/fc/vmlinux",
    "boot_args": "console=ttyS0 reboot=k panic=1 ro init=/sbin/pf-init"
  },
  "drives": [
    { "drive_id": "rootfs", "path_on_host": "/fc/rootfs.ext4", "is_root_device": true, "is_read_only": true }
  ],
  "machine-config": { "vcpu_count": 2, "mem_size_mib": 512 },
  "vsock": { "guest_cid": 3, "uds_path": "/fc/v.sock" }
}
JSON
for run in 1 2 3; do
rm -f /fc/v.sock
t0=$(now)
./firecracker --config-file /fc/config.json --no-api >/work/fc-console-$run.log 2>&1 </dev/null &
fcpid=$!
# Readiness: a bare CONNECT gets "OK <port>" once the guest listens
# (prototype polling; pinfold waits on its agent's ready signal).
for i in $(seq 1 600); do
  kill -0 $fcpid 2>/dev/null || { echo "firecracker exited"; break; }
  case $(printf 'CONNECT 1000\n' | socat -t0.1 - UNIX-CONNECT:/fc/v.sock 2>/dev/null) in OK*) break ;; esac
  sleep 0.05
done
t1=$(now)
# shut-none: Firecracker's vsock drops the stream on a host half-close, so
# socat must not shut down its write side at stdin EOF. The guest's `exit`
# closes the stream, which ends socat.
out=$(printf 'CONNECT 1000\nuname -a; head -3 /proc/cpuinfo; echo vsock-ok; exit\n' | socat -t10 - UNIX-CONNECT:/fc/v.sock,shut-none 2>/dev/null || true)
t2=$(now)
echo "== run $run guest reply"; printf '%s\n' "$out"
echo "run=$run boot_to_ready_ms=$(ms "$t0" "$t1") exec_ms=$(ms "$t1" "$t2") guest_init_at=$(grep -o '^\[ *[0-9.]*\] Run /sbin/pf-init' /work/fc-console-$run.log | tr -d '[]' | awk '{print $1}')s"
kill $fcpid; wait $fcpid 2>/dev/null || true
done
echo "== firecracker console (head)"; head -60 /work/fc-console-1.log
