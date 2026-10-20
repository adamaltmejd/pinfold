#!/bin/bash
# #138 labvm boot check: vmm install, root disk from an image, init disk, boot.
set -eux
export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
while sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 5; done
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq podman uidmap passt crun catatonit build-essential musl-tools git dbus-user-session e2fsprogs curl file >/dev/null
sudo loginctl enable-linger agent
systemctl --user enable --now dbus.socket
podman system migrate
sudo chmod o+rw /dev/kvm
[ -x ~/.cargo/bin/cargo ] || curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
. ~/.cargo/env
cd ~/pinfold
cargo clippy --all-targets --locked -q -- -D warnings && echo LINUX-CLIPPY-OK
patch -p1 < ~/scratch-disks.patch
rustup target add x86_64-unknown-linux-musl
cargo build -q -p pinfold --target x86_64-unknown-linux-musl
P=~/pinfold/target/x86_64-unknown-linux-musl/debug/pinfold
file $P
mkdir -p ~/ctx && printf 'FROM debian:trixie-slim\nRUN echo marker >/etc/boot-marker && chown 1234:2345 /etc/boot-marker\n' > ~/ctx/Containerfile
$P image build boot --containerfile ~/ctx/Containerfile --context ~/ctx
id=$(podman image inspect --format '{{.Id}}' localhost/pinfold/image-boot:latest)
mkdir -p ~/fc && rm -f ~/fc/init.ext4
time $P __disks "$id" ~/fc/init.ext4 | tee ~/fc/paths
read fc kernel root init < ~/fc/paths
$P artifacts | python3 -m json.tool | tail -20
ls -lsh "$root" "$init"; du -h --apparent-size "$root"
cmd='mount -t proc p /proc; mount -o ro -t ext4 /dev/vdb /mnt; echo uname=$(uname -r); /mnt/pinfold --version; echo marker-owner=$(stat -c %u:%g /etc/boot-marker); touch /rootwrite 2>&1 || echo root-readonly; touch /mnt/x 2>&1 || echo init-readonly; echo boot-check-done'
cat > ~/fc/vm.json <<J
{"boot-source":{"kernel_image_path":"$kernel","boot_args":"console=ttyS0 reboot=k panic=1 ro root=/dev/vda init=/bin/sh -- -c \"$cmd\""},
 "drives":[{"drive_id":"root","path_on_host":"$root","is_root_device":true,"is_read_only":true},
           {"drive_id":"init","path_on_host":"$init","is_root_device":false,"is_read_only":true}],
 "machine-config":{"vcpu_count":1,"mem_size_mib":256}}
J
cat ~/fc/vm.json
timeout 60 "$fc" --no-api --config-file ~/fc/vm.json > ~/fc/boot.txt 2>&1 || true
grep -vE '^\[ *[0-9]+\.[0-9]+\]' ~/fc/boot.txt | tail -30
# The merged Linux init (#151) as PID 1 from the init disk, the root disk as
# the box's root, driven by #151's throwaway host side.
F=~/pinfold/docs/archive/2026-10-20-linux-init-boot/fcinit.py
st=~/st; rm -rf $st; mkdir -p $st
timeout 120 python3 $F up $st/b --firecracker "$fc" --kernel "$kernel" --init "$init" --root "$root" >$st/b.up 2>&1 &
for i in $(seq 1 300); do grep -qE "ready|failed|Error|Traceback" $st/b.up 2>/dev/null && break; sleep 0.2; done
echo "up: $(cat $st/b.up)"
timeout 30 python3 $F exec $st/b -- /bin/sh -c 'echo uname=$(uname -r); echo marker-owner=$(stat -c %u:%g /etc/boot-marker); echo id=$(id -u):$(id -g); touch /etc/x 2>&1 || echo root-readonly; echo init-exec-done'; echo "[exec exit $?]"
timeout 30 python3 $F down $st/b; echo "[down exit $?]"
sleep 2; grep -v '^\[ *[0-9]' $st/b/console.log | tail -15
$P image rm boot
ls -la ~/.cache/pinfold/root/
echo LABVM-DONE
