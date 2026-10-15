#!/bin/sh
set -eu

# AppArmor's unprivileged-userns restriction is enabled on the runner image
# and denies rootless podman the user namespace it creates. The runner is
# disposable, so clearing it is enough.
if [ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
fi

# podman needs an active systemd user session to enforce --cpus and
# --memory; under cgroupfs it silently ignores both and pinfold's preflight
# refuses every box. The image enables linger, but not the session D-Bus.
sudo apt-get update -qq
sudo apt-get install -y -qq dbus-user-session build-essential pkg-config \
  libapparmor-dev libbtrfs-dev libgpgme-dev libseccomp-dev libsqlite3-dev \
  libsubid-dev libsystemd-dev fuse-overlayfs

# The runner's packaged Podman is older than pinfold's minimum. Build the
# unmodified release: upstream's Linux release binaries are remote clients.
podman_commit=464c15fec06f0074c916f9eff87c4806869c2833 # v6.1.4
podman_source=$(mktemp -d)
trap 'rm -rf "$podman_source"' EXIT
trap 'exit 1' HUP INT TERM
git clone --quiet --depth 1 --branch v6.1.4 \
  https://github.com/podman-container-tools/podman.git "$podman_source"
test "$(git -C "$podman_source" rev-parse HEAD)" = "$podman_commit"
# Keep upstream's native build tags, including seccomp, AppArmor and systemd.
# Use the distribution's helper binaries and configuration paths.
make -C "$podman_source" bin/podman PREFIX=/usr
sudo install -m 0755 "$podman_source/bin/podman" /usr/local/bin/podman
# Older vendor storage configs override the new runtime's rootless paths.
sudo install -m 0644 "$podman_source/vendor/go.podman.io/storage/storage.conf" \
  /usr/share/containers/storage.conf
# Probe-only selection precedes every Podman command and uses the normal
# per-user store only when it is absent or empty. No existing store is reset.
CONTAINERS_STORAGE_CONF=$(python3 scripts/configure-fuse-probe.py)
export CONTAINERS_STORAGE_CONF
export PATH="/usr/local/bin:$PATH"
echo /usr/local/bin >>"$GITHUB_PATH"
test "$(podman --version)" = "podman version 6.1.4"

sudo loginctl enable-linger "$(id -un)"
sudo systemctl start "user@$(id -u).service"

XDG_RUNTIME_DIR="/run/user/$(id -u)"
DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
export XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
systemctl --user daemon-reload
systemctl --user enable --now dbus.socket
podman system migrate

{
  echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
  echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
} >>"$GITHUB_ENV"
