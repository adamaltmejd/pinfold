#!/bin/sh
# Prepare a fresh Debian 13 amd64 labvm lease: rootless podman 6.1.4 (as
# scripts/setup-linux-runner.sh builds it), rust, socat and e2fsprogs.
set -eu
while sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 2; done
sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  podman uidmap passt crun catatonit conmon build-essential musl-tools git \
  dbus-user-session pkg-config libapparmor-dev libbtrfs-dev libgpgme-dev \
  libseccomp-dev libsqlite3-dev libsubid-dev libsystemd-dev \
  socat e2fsprogs curl jq acl time >/dev/null
# Go from go.dev: trixie's 1.24 is older than podman 6's go.mod.
go_version=1.25.3
curl -fsSL "https://go.dev/dl/go$go_version.linux-amd64.tar.gz" -o /tmp/go.tgz
sudo rm -rf /usr/local/go && sudo tar -C /usr/local -xzf /tmp/go.tgz
export PATH=/usr/local/go/bin:$PATH
src=$(mktemp -d)
git clone --quiet --depth 1 --branch v6.1.4 \
  https://github.com/podman-container-tools/podman.git "$src"
test "$(git -C "$src" rev-parse HEAD)" = 464c15fec06f0074c916f9eff87c4806869c2833
make -C "$src" bin/podman PREFIX=/usr >/dev/null
sudo install -m 0755 "$src/bin/podman" /usr/local/bin/podman
sudo install -m 0644 "$src/vendor/go.podman.io/storage/storage.conf" \
  /usr/share/containers/storage.conf
sudo loginctl enable-linger "$(id -un)"
sudo systemctl start "user@$(id -u).service"
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
systemctl --user enable --now dbus.socket
/usr/local/bin/podman system migrate
/usr/local/bin/podman --version
