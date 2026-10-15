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
  libsubid-dev libsystemd-dev

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
export PATH="/usr/local/bin:$PATH"
echo /usr/local/bin >>"$GITHUB_PATH"
# Hosted x64 runners own this empty store until the disposable VM is removed.
pinfold_ci_tmpfs=false
if [ "${GITHUB_ACTIONS:-}" = true ] && [ "${RUNNER_ENVIRONMENT:-}" = github-hosted ] && [ "$(uname -m)" = x86_64 ]; then
  pinfold_ci_tmpfs=true
  python3 - <<'PY_PREPARE'
import json
import os
from pathlib import Path
import stat
import subprocess

home = Path.home()
graph = home / ".local/share/containers/storage"
assert os.getuid() != 0 and home.is_absolute() and graph.resolve() == graph, "unexpected rootless store path"
assert not os.environ.get("XDG_DATA_HOME") and not os.environ.get("CONTAINERS_STORAGE_CONF"), "storage override present"
assert not (home / ".config/containers/storage.conf").exists(), "user storage configuration present"
mounted = subprocess.run(["findmnt", "--mountpoint", str(graph), "--noheadings"], capture_output=True, timeout=10)
assert mounted.returncode == 1 and not mounted.stdout and not mounted.stderr, "store already mounted or mount inventory failed"
if graph.exists():
    metadata = graph.stat()
    assert graph.is_dir() and not any(graph.iterdir()), "store is not empty"
    assert (metadata.st_uid, metadata.st_gid) == (os.getuid(), os.getgid()), "store ownership differs"
    assert stat.S_IMODE(metadata.st_mode) == 0o700, "store permissions differ"
else:
    graph.mkdir(mode=0o700, parents=True)
options = f"size=6G,mode=0700,uid={os.getuid()},gid={os.getgid()}"
subprocess.run(["sudo", "mount", "-t", "tmpfs", "-o", options, "pinfold-ci-tmpfs", str(graph)], check=True, timeout=30)
mounted = json.loads(subprocess.check_output(["findmnt", "--json", "--mountpoint", str(graph), "-o", "TARGET,FSTYPE,SOURCE"], timeout=10))["filesystems"]
assert len(mounted) == 1 and mounted[0] == {"target": str(graph), "fstype": "tmpfs", "source": "pinfold-ci-tmpfs"}, "unexpected store mount"
metadata = graph.stat()
assert (metadata.st_uid, metadata.st_gid, stat.S_IMODE(metadata.st_mode)) == (os.getuid(), os.getgid(), 0o700)
assert os.statvfs(graph).f_blocks * os.statvfs(graph).f_frsize == 6 * 1024**3, "store cap differs"
PY_PREPARE
fi
test "$(podman --version)" = "podman version 6.1.4"

sudo loginctl enable-linger "$(id -un)"
sudo systemctl start "user@$(id -u).service"

XDG_RUNTIME_DIR="/run/user/$(id -u)"
DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
export XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
systemctl --user daemon-reload
systemctl --user enable --now dbus.socket
podman system migrate

if [ "$pinfold_ci_tmpfs" = true ]; then
  python3 - <<'PY_VERIFY'
import json
import os
from pathlib import Path
import subprocess

info = json.loads(subprocess.check_output(["podman", "info", "--format", "json"], timeout=90))
store = info["store"]
assert info["host"]["security"]["rootless"] is True
assert store["graphRoot"] == str(Path.home() / ".local/share/containers/storage")
assert store["runRoot"] == f"/run/user/{os.getuid()}/containers"
assert store["graphDriverName"] == "overlay" and not store["transientStore"]
assert store["graphOptions"].get("overlay.mountopt") == "nodev"
assert not store["graphOptions"].get("overlay.mount_program"), "FUSE fallback refused"
assert store["graphStatus"]["Native Overlay Diff"] == "true"
assert store["graphStatus"]["Backing Filesystem"] == "tmpfs"
assert store["graphRootAllocated"] == 6 * 1024**3, "store cap differs"
PY_VERIFY
fi

{
  echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
  echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
} >>"$GITHUB_ENV"
