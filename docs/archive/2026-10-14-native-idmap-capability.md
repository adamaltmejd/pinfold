# Native ID-mapping capability on yard-sthlm

Stock Podman 6.1.4 cannot create an ID-mapped mount of its ext4 storage
inside Yard's rootless user namespace. The failing operation is
`mount_setattr(..., MOUNT_ATTR_IDMAP)`, returning `EPERM`. The same
operation succeeds on a tiny tmpfs created inside that namespace.

This closes the proposed native-shifting configuration avenue for the
current storage setup. It does not fix issue #89. No Pinfold implementation,
runtime configuration, security policy or active storage was changed.

## Observed boundary

Host: `yard-sthlm`, unprivileged LXC, kernel `7.0.14-19-pve`.
The enclosing LXC UID/GID maps both read `0 100000 65536`.
Podman binary SHA-256:
`11326740634a0929f438256c4e3ff90273c4399a279c68e9fde02be25f75c2a5`.
Its upstream commit is `464c15fec06f0074c916f9eff87c4806869c2833`.

A fresh disposable graph/run root prevented reuse of a cached negative
capability result. Effective paths were checked. `podman --log-level=debug
info --format json` exited zero and reported native overlay, empty graph
options, `Backing Filesystem: extfs`, and `Supports shifting: false`.
Its capability check logged:

```text
Check for idmapped mounts support create mapped mount: mount_setattr .../overlay/compat15033875/lower: operation not permitted
```

The failure occurs before the final overlay mount. It is not evidence of
an unsupported overlay mount option or a Pinfold seccomp rejection: this
is the runtime's own host-side storage initialization.

## Controlled syscall probe

A small C program ran through the same stock `podman unshare` using the
private configuration. It created a private mount namespace, a child user
namespace with UID/GID mappings `0 0 1`, and detached mount clones using
`open_tree`. ID-mapping used the same flags and attributes as Podman's
helper: `AT_EMPTY_PATH|AT_RECURSIVE`, `MOUNT_ATTR_IDMAP`, `MS_PRIVATE`, and
the child user-namespace descriptor.

It compared a directory on the existing ext4 filesystem with a new tmpfs
mounted inside the Podman namespace. The tmpfs was limited to 1 MiB and
mounted `nodev,nosuid,mode=0700`. A separate detached clone of each source
received an ordinary `MOUNT_ATTR_NOSUID` change as a positive control.

| Source | Attribute | Result |
|---|---|---|
| Existing ext4 | NOSUID | success |
| Existing ext4 | IDMAP | EPERM |
| Namespace-owned tmpfs | NOSUID | success |
| Namespace-owned tmpfs | IDMAP | success |

The initial probe and a confirmation with explicit exit recording agreed.
The confirmed program exited zero with empty stderr. It compiled with
`-Wall -Wextra -Werror`. Its child was killed and reaped; the private tmpfs
was detached and both temporary directories removed before exit.
This checks the syscall boundary, not full container or workload behavior.

Source SHA-256:
`40c57a97c39458ff8f21bf6e9a66360e859b6d76804677d8f3adba8ebec301e9`.
Probe binary SHA-256:
`0184ed3317b3eecf3175eed3f505052c9c785edfc34a46fb871cbeb95bcae045`.

## Explanation and implications

Linux requires `CAP_SYS_ADMIN` in the user namespace that owns the
filesystem to create an ID-mapped mount. Being root inside Podman's child
namespace does not grant that capability over the existing host-backed
ext4 filesystem. A new tmpfs belongs to the namespace that mounted it,
which explains the successful control.

This is the supported explanation from the controls and upstream kernel
source. No kernel instrumentation identified the exact branch in the PVE
build, so an errno alone is not claimed to prove a particular kernel line.
The successful tmpfs control excludes a blanket prohibition of the tested
IDMAP syscall and demonstrates working namespace/mapping setup.

Relevant primary sources:

- [Stock Podman capability probe](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/vendor/go.podman.io/storage/drivers/overlay/check.go)
- [Stock ID-mapped mount helper](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/vendor/go.podman.io/storage/pkg/idmap/idmapped_utils.go)
- [Linux mount_setattr requirements](https://man7.org/linux/man-pages/man2/mount_setattr.2.html)
- [Upstream Linux 7.0.14 capability check](https://github.com/gregkh/linux/blob/v7.0.14/fs/namespace.c#L4761-L4801)

A newer kernel, a generic move to a VM, or a Podman option is not a
demonstrated solution to this boundary. The tmpfs control is not a proposal
to move persistent image storage into RAM. Granting host privileges,
introducing privileged mount management, or changing storage architecture
would exceed the current constraints. No simple stock-rootless persistent
storage switch was established.

Keep Pinfold's bounded truthful inventory behavior. No complete fix within
the current constraints was established. Correcting the demonstrated
long-held storage locks would require a supported runtime change.
Exact-image preparation remains mitigation only, as the previous
derived-image reproduction established. No upstream issue, comment or PR
was published.

## Cleanup and evidence

The whole experiment occupied 229,376 bytes before cleanup. A fresh
root-readable scan found no graph/run/probe process or mount references.
The private graph and run roots were removed. About 68 KiB of evidence
remained, plus the compressed archive. No images were pulled or built.

Remote evidence:
`/home/yard/validation/pinfold-89-idmap-20261014`.
Local archive:
`/private/tmp/pinfold89-idmap-evidence-20261014.tar.gz`.
Archive SHA-256, verified after transfer:
`a87eb679ef86ca17c59233d8aaca38df4af5409187dbb4c540f698dc464a3cff`.
