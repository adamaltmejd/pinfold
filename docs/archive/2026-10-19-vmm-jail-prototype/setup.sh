#!/bin/sh
# Prepare a fresh Debian 13 amd64 labvm lease: kvm group, Rust (musl),
# Firecracker 1.17.0 and the CI guest kernel (as PR #111), and a debootstrap
# guest root. No podman: this prototype uses a stand-in proxy.
set -eu
while sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 2; done
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  build-essential musl-tools socat e2fsprogs curl jq debootstrap python3 >/dev/null
sudo usermod -aG kvm "$(id -un)"
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable -t x86_64-unknown-linux-musl >/dev/null 2>&1
fc=v1.17.0
kernel=firecracker-ci/20261016-5732b8331e1e-0/x86_64/vmlinux-6.1.188
mkdir -p ~/fc && cd ~/fc
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-x86_64.tgz"
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-x86_64.tgz.sha256.txt"
sha256sum -c "firecracker-$fc-x86_64.tgz.sha256.txt"
tar -xzf "firecracker-$fc-x86_64.tgz"
install -m 0755 "release-$fc-x86_64/firecracker-$fc-x86_64" firecracker
curl -fsSL -o vmlinux "https://s3.amazonaws.com/spec.ccfc.min/$kernel"
sha256sum firecracker vmlinux
./firecracker --version | head -1
sudo debootstrap --variant=minbase --include=curl,ca-certificates,socat,procps \
  trixie /srv/guestroot http://deb.debian.org/debian >/dev/null
echo setup-done
