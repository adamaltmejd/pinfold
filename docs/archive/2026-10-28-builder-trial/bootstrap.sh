#!/bin/bash
# Reproduction recipe for the disposable trial, not an installed Pinfold path.
# Run only inside an exclusively leased labvm; release it in a finally block.
set -euo pipefail
cd /home/agent
sudo apt-get update -qq
sudo apt-get -o DPkg::Lock::Timeout=120 install -y -qq \
  debootstrap busybox-static runc e2fsprogs socat ca-certificates iproute2
sudo usermod -aG kvm agent
mkdir -p phase6-trial
cd phase6-trial
# Copy the trial's .py files and trial-init here before the final root assembly.
curl -fsSL --proto '=https' -o buildkit.tar.gz https://github.com/moby/buildkit/releases/download/v0.34.1/buildkit-v0.34.1.linux-amd64.tar.gz
echo 'a59539daf547fe0958efe2d07e41c0a6944e1112a66d7e519ec1cf5b673ef15b  buildkit.tar.gz' | sha256sum -c -
curl -fsSL --proto '=https' -o firecracker.tgz https://github.com/firecracker-microvm/firecracker/releases/download/v1.17.0/firecracker-v1.17.0-x86_64.tgz
echo '06094a1108ae9e82aa4c23a775aa92758f53f1175d422270d9d6162cb9ade558  firecracker.tgz' | sha256sum -c -
curl -fsSL --proto '=https' -o vmlinux https://github.com/adamaltmejd/pinfold/releases/download/guest-kernel-6.18.57-firecracker-v1.17.0/vmlinux-6.18.57
echo '1a1b28b34f89a8e49d90c72997f1b89bfd236ff44669dda9e91486c89adafc99  vmlinux' | sha256sum -c -
tar -xzf firecracker.tgz
tar -xzf buildkit.tar.gz
sudo debootstrap --variant=minbase \
  --include=python3,socat,runc,ca-certificates,iproute2,busybox-static,curl,e2fsprogs,libcap2-bin,git \
  trixie root https://deb.debian.org/debian >debootstrap.log 2>&1
sudo cp -a bin/* root/usr/local/bin/
sudo install -m 0755 trial-init root/sbin/trial-init
sudo install -m 0755 trial-guest.py root/usr/local/lib/trial-guest.py
sudo install -m 0755 frames.py root/usr/local/lib/frames.py
sudo mkdir -p root/var/lib/buildkit root/run root/tmp root/proc root/sys root/dev/pts root/dev/shm root/input
sudo sh -c ': > root/etc/resolv.conf'
sudo chroot root dpkg-query -W >bootstrap-packages.tsv
truncate -s 1G root.ext4
sudo /usr/sbin/mkfs.ext4 -q -F -d root root.ext4
truncate -s 8G cache.ext4
/usr/sbin/mkfs.ext4 -q -F cache.ext4
sha256sum root.ext4 >bootstrap-root.sha256
# Reconnect via a later labvm exec so the newly assigned KVM group is active.
