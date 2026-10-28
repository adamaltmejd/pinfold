# Manual ACL converter component proof

`acl-proof.py` constructs two OCI layers from disposable host files whose raw
ACL and capability attributes come from the host kernel. It copies the base
image's validated blobs without extracting them. It boots the compiled
production converter guest, receives its opaque root image, then boots that
image with the compiled production agent and checks the metadata as the host
user's UID and GID.

This is component runtime evidence only. It bypasses the normal host CLI,
Firecracker jail, common claims and production output receiver. Success does
not establish those boundaries or ordinary build export's ACL fidelity.

Run as the unprivileged user of an exclusively leased Linux/KVM host. Python
3.11 or later, `mkfs.ext4`, `setfacl`, `getfacl`, `setcap`, and passwordless
`sudo` for `chown` and `setcap` are required. The supplied base must be a
validated, single runnable amd64 OCI layout containing `getfacl`, `getfattr`,
`getcap`, `stat`, `readlink`, and `test`. The script installs nothing and uses
no network. Supply the release archive because `vmm.toml` pins that archive,
rather than the extracted Firecracker executable.

```sh
python3 docs/archive/2026-10-28-builder-integration/acl-proof.py \
  --binary /absolute/path/to/compiled-musl-pinfold \
  --builder-root /absolute/path/to/pinned-builder-root.ext4 \
  --firecracker-archive /absolute/path/to/pinned-firecracker.tgz \
  --kernel /absolute/path/to/pinned-vmlinux \
  --base-layout /absolute/path/to/validated-metadata-oci \
  --work /tmp/acl-proof-new
```

The work directory must be new and short enough for a Unix socket path.
Keep it with the supplied immutable artifacts. It records the original base
metadata, constructed OCI input, raw host expectations, pinned asset metadata,
input disks, converted root, console logs, and each guest command's output.
`receipt.json` exists only after all checks pass. `failure.json` records an
incomplete attempt. The guest-writable work disk is deleted only after success;
the host never reads or mounts it. Both VMM processes are killed and reaped
when their respective scope ends.

The harness has no sleeps or retries. It waits for the production BOOT and
agent replies, with a five-minute deadline per guest. This source was added
without executing the proof.
