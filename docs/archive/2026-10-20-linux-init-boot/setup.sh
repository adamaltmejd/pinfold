#!/bin/bash
# Fresh labvm lease: tools, Rust, Firecracker v1.17.0 and pinfold's guest
# kernel, both checked against their published digests.
set -euo pipefail
while sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 2; done
sudo apt-get update -q
sudo apt-get install -yq build-essential musl-tools git curl debootstrap e2fsprogs python3
sudo usermod -aG kvm "$USER"
[ -x ~/.cargo/bin/rustup ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
mkdir -p ~/fc && cd ~/fc
v=v1.17.0
curl -sfLO "https://github.com/firecracker-microvm/firecracker/releases/download/$v/firecracker-$v-x86_64.tgz"
curl -sfLO "https://github.com/firecracker-microvm/firecracker/releases/download/$v/firecracker-$v-x86_64.tgz.sha256.txt"
sha256sum -c "firecracker-$v-x86_64.tgz.sha256.txt"
tar -xzf "firecracker-$v-x86_64.tgz"
cp "release-$v-x86_64/firecracker-$v-x86_64" firecracker
curl -sfL -o vmlinux https://github.com/adamaltmejd/pinfold/releases/download/guest-kernel-6.18.57-firecracker-v1.17.0/vmlinux-6.18.57
echo "1a1b28b34f89a8e49d90c72997f1b89bfd236ff44669dda9e91486c89adafc99  vmlinux" | sha256sum -c
