#!/bin/bash
set -euo pipefail
sudo apt-get -o DPkg::Lock::Timeout=600 update
sudo apt-get -o DPkg::Lock::Timeout=600 install -y build-essential musl-tools git rustup gdb curl ca-certificates e2fsprogs
sudo chgrp "$(id -gn)" /dev/kvm
sudo chmod 660 /dev/kvm
git clone -q https://github.com/adamaltmejd/pinfold.git ~/pinfold
cd ~/pinfold
git checkout -q --detach 809fc36
git apply /tmp/pfo-source.patch
rustup target add x86_64-unknown-linux-musl
cargo build --locked --target x86_64-unknown-linux-musl
mkdir -p /tmp/pfo-image /tmp/pfo-imgstate/pinfold /tmp/pfo-imgconfig
: > /tmp/pfo-imgstate/pinfold/maintenance
cat > /tmp/pfo-image/Containerfile <<'CF'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends git curl perl ca-certificates && rm -rf /var/lib/apt/lists/*
CF
df -h /
free_gib=$(df -BG / | awk 'NR==2 {gsub(/G/,"",$4); print $4}')
[ "$free_gib" -ge 12 ]
export XDG_CONFIG_HOME=/tmp/pfo-imgconfig XDG_STATE_HOME=/tmp/pfo-imgstate PINFOLD_NO_UPDATE_CHECK=1
exec target/x86_64-unknown-linux-musl/debug/pinfold image build probe-owner-check --containerfile /tmp/pfo-image/Containerfile --context /tmp/pfo-image --allow deb.debian.org --allow security.debian.org
