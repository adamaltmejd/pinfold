#!/bin/bash
# build.sh SRC NAME: build SRC's pinfold for x86_64 musl and wrap it in the
# init disk ~/fc/NAME.ext4 (/pinfold, empty /dev and /mnt). Also makes the
# root disk once: a Debian trixie minbase with python3 and no setuid files.
set -euo pipefail
src=$1 name=$2
export PATH=$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin
(cd "$src" && rustup target add x86_64-unknown-linux-musl >/dev/null &&
  cargo build -q --release --locked -p pinfold --target x86_64-unknown-linux-musl)
d=$(mktemp -d)
install -m 0755 "${CARGO_TARGET_DIR:-$src/target}/x86_64-unknown-linux-musl/release/pinfold" "$d/pinfold"
mkdir "$d/dev" "$d/mnt"
rm -f ~/fc/"$name".ext4
mkfs.ext4 -q -d "$d" ~/fc/"$name".ext4 64M
rm -rf "$d"
if [ ! -f ~/fc/root.ext4 ]; then
  # Not /tmp: debootstrap makes device nodes, which a nodev /tmp refuses.
  r=/var/tmp/pinfold-root
  sudo rm -rf "$r"
  sudo debootstrap --variant=minbase --include=python3-minimal,procps trixie "$r" >/dev/null
  sudo find "$r" -xdev -perm /6000 -type f -exec chmod ug-s {} +
  sudo mkfs.ext4 -q -d "$r" ~/fc/root.ext4 1G
  sudo chown "$USER" ~/fc/root.ext4
  sudo rm -rf "$r"
fi
