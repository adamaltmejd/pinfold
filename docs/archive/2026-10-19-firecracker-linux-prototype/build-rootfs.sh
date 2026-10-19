#!/bin/sh
# Convert pinfold's profile image into a Firecracker root disk:
# profile image -> one guest layer (guest.Containerfile) -> podman export ->
# ext4 via mkfs.ext4 -d, run inside `podman unshare` so file owners keep
# their in-image uids. Usage: build-rootfs.sh [PROFILE_IMAGE]
set -eu
profile=${1:-pinfold/profile-default:latest}
here=$(cd "$(dirname "$0")" && pwd)
ctx=$(mktemp -d)
. "$HOME/.cargo/env"
(cd "$HOME/pinfold" && rustc --edition 2024 -O --target x86_64-unknown-linux-musl -o "$ctx/pf-agent" "$here/pf-agent.rs")
cp "$ctx/pf-agent" "$HOME/fc/pf-agent"
cp "$here/pf-init" "$here/pf-shell" "$ctx/"
t0=$(date +%s)
podman build -q --build-arg "PROFILE=$profile" -f "$here/guest.Containerfile" -t pinfold-fc-guest "$ctx"
t1=$(date +%s)
cid=$(podman create pinfold-fc-guest)
rm -rf "$HOME/fc/rootfs.d"
podman unshare sh -c "mkdir $HOME/fc/rootfs.d && podman export $cid | tar -x -C $HOME/fc/rootfs.d"
podman rm -f "$cid" >/dev/null
kib=$(podman unshare du -sk "$HOME/fc/rootfs.d" | cut -f1)
podman unshare mkfs.ext4 -q -F -d "$HOME/fc/rootfs.d" "$HOME/fc/rootfs.ext4" "$(( kib * 6 / 5 + 65536 ))K"
podman unshare rm -rf "$HOME/fc/rootfs.d"
t2=$(date +%s)
echo "guest_layer_s=$((t1-t0)) export_mkfs_s=$((t2-t1)) rootfs_kib=$kib"
ls -l "$HOME/fc/rootfs.ext4"
