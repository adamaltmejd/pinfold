#!/bin/sh
# Fetch the pinned Firecracker release (firecracker + jailer) and the guest
# kernel Firecracker's own CI builds. Prints sha256 of each artifact.
set -eu
fc=v1.17.0
kernel=firecracker-ci/20261016-5732b8331e1e-0/x86_64/vmlinux-6.1.188
out=~/fc && mkdir -p "$out" && cd "$out"
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-x86_64.tgz"
curl -fsSL -O "https://github.com/firecracker-microvm/firecracker/releases/download/$fc/firecracker-$fc-x86_64.tgz.sha256.txt"
sha256sum -c "firecracker-$fc-x86_64.tgz.sha256.txt"
tar -xzf "firecracker-$fc-x86_64.tgz"
install -m 0755 "release-$fc-x86_64/firecracker-$fc-x86_64" firecracker
install -m 0755 "release-$fc-x86_64/jailer-$fc-x86_64" jailer
curl -fsSL -o vmlinux "https://s3.amazonaws.com/spec.ccfc.min/$kernel"
sha256sum firecracker jailer vmlinux "firecracker-$fc-x86_64.tgz"
./firecracker --version | head -1
