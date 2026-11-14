#!/bin/bash
# Standalone Firecracker diagnostic, outside pinfold's jail. Not a box run.
set -euo pipefail
BIN=$(realpath "$1")
W=$(mktemp -d /tmp/pfh.XXXX)
trap 'rm -rf "$W"' EXIT
scrub() {
    sed -E -e "s#$W#\$W#g" -e "s#$HOME#/home/user#g" \
        -e "s#$(id -un)#user#g" \
        -e 's#(Linux version [^ ]+) \([^)]*\)#\1 (user@build-host)#'
}
exec > >(scrub) 2>&1
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
mkdir -p "$XDG_STATE_HOME/pinfold" "$W/tree/dev" "$W/tree/mnt"
touch "$XDG_STATE_HOME/pinfold/maintenance"
echo '$ file BIN; readelf -l BIN (interpreter)'
file "$BIN"
readelf -l "$BIN" | sed -n '/interpreter/p'
echo '$ pinfold doctor (preflight lines)'
"$BIN" doctor | sed -n '/runtime/Ip; /preflight/Ip; /kvm/Ip'
VMM=$HOME/.cache/pinfold/artifacts/vmm/1.17.0+6.18.57/linux-x64
FC=$(find "$VMM/firecracker" -type f -executable -name 'firecracker*' | head -1)
ROOT=$(find "$HOME/.cache/pinfold/artifacts/builder" -path '*/linux-x64/root.ext4' | head -1)
MKFS=$(command -v mkfs.ext4 || printf /usr/sbin/mkfs.ext4)
cp "$BIN" "$W/tree/pinfold"
echo '$ mkfs.ext4: init with only pinfold, dev, mnt; separate work/input/selected'
"$MKFS" -q -F -m 0 -O ^has_journal -d "$W/tree" "$W/init.ext4" 262144
for disk in work input selected; do
    truncate -s 32M "$W/$disk.ext4"
    "$MKFS" -q -F -m 0 "$W/$disk.ext4"
done
python3 - "$W" "$VMM/vmlinux" "$ROOT" <<'PY'
import json, sys
from pathlib import Path
w, kernel, root = sys.argv[1:]
drives = [("init", f"{w}/init.ext4", True, True), ("root", root, False, True)]
drives += [(f"disk{i}", f"{w}/{name}.ext4", False, name != "work")
           for i, name in enumerate(["work", "input", "selected"])]
config = {
    "boot-source": {"kernel_image_path": kernel, "boot_args":
        "console=ttyS0 reboot=k panic=1 pci=off init=/pinfold -- init builder"},
    "drives": [dict(drive_id=i, path_on_host=p, is_root_device=r, is_read_only=ro)
               for i, p, r, ro in drives],
    "machine-config": {"vcpu_count": 2, "mem_size_mib": 2560},
    "vsock": {"guest_cid": 3, "uds_path": f"{w}/v.sock"},
}
Path(w, "config.json").write_text(json.dumps(config))
print(json.dumps(config, indent=2))
PY
echo '$ timeout 30 Firecracker --config-file "$W/config.json" --no-api'
set +e
timeout 30 env -i "$FC" --config-file "$W/config.json" --no-api </dev/null
echo "[exit $?]"
