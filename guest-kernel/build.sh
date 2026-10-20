#!/bin/sh
# Builds pinfold's Firecracker guest kernel into the directory $1:
# vmlinux-$KERNEL and SHA256SUMS. Pins are in ./pins, Firecracker's version
# in vmm.toml.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
. "$here/pins"
FIRECRACKER=v$(sed -n 's/^version = "\([^+"]*\)+[^"]*"$/\1/p' "$here/../crates/pinfold/vmm.toml")
# Fixed build metadata, so a rebuild can be compared with the pinned digest.
export KBUILD_BUILD_TIMESTAMP='1970-01-01 00:00:00 UTC'
export KBUILD_BUILD_USER=pinfold KBUILD_BUILD_HOST=pinfold
mkdir -p "$1"
out=$(cd "$1" && pwd)
series=${KERNEL%.*}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

fetch() {
	curl -fsSL --proto '=https' --max-redirs 0 -o "$1" "$2"
}

# kernel.org signs the uncompressed tarball. gregkh.asc holds only the
# stable release key, 647F28654894E3BD457199BE38DBBDC86092693E.
url=https://cdn.kernel.org/pub/linux/kernel/v${KERNEL%%.*}.x/linux-$KERNEL
fetch linux.tar.xz "$url.tar.xz"
fetch linux.tar.sign "$url.tar.sign"
echo "$KERNEL_SHA256  linux.tar.xz" | sha256sum -c --quiet -
xz -d linux.tar.xz
gpg --dearmor <"$here/gregkh.asc" >"$work/gregkh.gpg"
GNUPGHOME=$work gpgv --keyring "$work/gregkh.gpg" linux.tar.sign linux.tar
tar -xf linux.tar
cd "linux-$KERNEL"

fetch .config "https://raw.githubusercontent.com/firecracker-microvm/firecracker/$FIRECRACKER/resources/guest_configs/microvm-kernel-ci-x86_64-$series.config"
echo "$CONFIG_SHA256  .config" | sha256sum -c --quiet -
# Spec: the config at /proc/config.gz, no vsock loopback (guarantee 1).
./scripts/config -e IKCONFIG -e IKCONFIG_PROC -d VSOCKETS_LOOPBACK
make -s olddefconfig
grep -qx 'CONFIG_IKCONFIG_PROC=y' .config
grep -qx '# CONFIG_VSOCKETS_LOOPBACK is not set' .config
make -s -j"$(nproc)" vmlinux

cp vmlinux "$out/vmlinux-$KERNEL"
cd "$out"
sha256sum "vmlinux-$KERNEL" >SHA256SUMS
