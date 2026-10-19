#!/bin/sh
# Build fcjail and pf-guest (static musl), the guest root disk from the
# debootstrap tree, and the init disk.
set -eu
export PATH=$PATH:/usr/sbin:/sbin
. "$HOME/.cargo/env"
cd "$HOME/proto"
for p in fcjail pf-guest; do
  rustc --edition 2024 -O --target x86_64-unknown-linux-musl -o "$HOME/fc/$p" "$p.rs"
done
sudo install -m 0755 "$HOME/fc/pf-guest" /srv/guestroot/sbin/pf-guest
sudo mkdir -p /srv/guestroot/data
sudo find /srv/guestroot -xdev -perm /6000 -type f -exec chmod a-s {} +
sudo mkfs.ext4 -q -F -d /srv/guestroot "$HOME/fc/rootfs.ext4" 512M
sudo chown "$(id -u):$(id -g)" "$HOME/fc/rootfs.ext4"
d=$(mktemp -d); mkdir -p "$d/sbin"; cp "$HOME/fc/pf-guest" "$d/sbin/"
mkfs.ext4 -q -F -d "$d" "$HOME/fc/init.ext4" 8M
chmod 0444 "$HOME/fc/rootfs.ext4" "$HOME/fc/init.ext4"
ls -l "$HOME/fc"
