#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
t0=$(date +%s)
apt-get update -qq
apt-get install -y -qq --no-install-recommends build-essential flex bison bc libssl-dev libelf-dev curl ca-certificates xz-utils cpio python3 >/dev/null
cd /root
curl -fsSLO https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.15.tar.xz
curl -fsSL https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc | grep ' linux-6.18.15.tar.xz$' | sha256sum -c -
tar xf linux-6.18.15.tar.xz
cd linux-6.18.15
cp /work/apple-6.18.15.config .config
./scripts/config -e VIRTUALIZATION -e KVM
make olddefconfig >/dev/null
grep -E '^(# )?CONFIG_(VIRTUALIZATION|KVM|ARM64_VHE)[ =]' .config | tee /work/kvm-options.txt
grep -q '^CONFIG_KVM=y' .config
cp .config /work/kvm.config
t1=$(date +%s)
make -j"$(nproc)" Image >/work/make.log 2>&1
cp arch/arm64/boot/Image /work/Image-6.18.15-kvm
t2=$(date +%s)
echo "setup+fetch $((t1-t0))s build $((t2-t1))s nproc $(nproc)" | tee /work/timing.txt
