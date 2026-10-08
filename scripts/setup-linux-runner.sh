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
sudo apt-get install -y -qq dbus-user-session
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
