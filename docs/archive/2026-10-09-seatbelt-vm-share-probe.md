# Seatbelt enforcement through a VM share

## Result

Applying a Seatbelt write-denial policy to the process that creates an
Apple Virtualization VM did not restrict guest writes through its writable
directory share in this experiment. The process itself could not write
the protected paths, but an unprivileged guest could write the exact same
host files. A policy denying all writes to the shared directory also
failed to restrict guest writes.

Setting `VZSharedDirectory.readOnly` to true did block every tested guest
write, including the ordinary-file control. Thus the test distinguishes
an enforced read-only export from an ineffective caller Seatbelt rule.

This rejects the proposed shortcut of adding Seatbelt to the VM-owning
helper and relying on its path rules to protect writable worktree exports.
It does not prove that a different filesystem service with its own
enforcement cannot work. Nor does it establish an Apple vulnerability:
the launcher explicitly grants a writable directory share, and this
experiment shows that the caller's additional path denials do not narrow
that grant in the tested arrangement.

## Setup

Run on 2026-10-09, arm64 macOS 27.0.1. A standalone Swift executable used
Apple's Virtualization framework. It was signed with the same virtualization
entitlement used by the installed Apple Container helper. The executable
ran under `sandbox-exec` before constructing any VZ objects or opening the
share through VZ.

The share construction matched
[Containerization 0.47.0](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/VZVirtualMachineInstance.swift#L494),
which is pinned by Apple Container 1.5.0:

```swift
let share = VZVirtioFileSystemDeviceConfiguration(tag: "virtiofs")
share.share = VZMultipleDirectoryShare(directories: [
    "fixture": VZSharedDirectory(url: fixtureURL, readOnly: false)
])
config.directorySharingDevices = [share]
```

The VM used the existing cached Linux kernel, `vmlinux-6.18.15-186`, with
a small temporary initramfs. BusyBox and its musl loader came from the
already cached image
`docker.io/oven/bun@sha256:d888c0ae6c86d7866ff10c5aafdd9077b36aee6455b33dd270fb93c0dd5cef6f`.
A temporary network-disabled container extracted those files and was
removed. No image was built and no installed runtime configuration changed.

Each VM had 2 CPUs, 512 MiB RAM, no network device, and only its disposable
fixture directory shared. Initial root setup mounted the share with
`nosuid,nodev`, then ran the probe as uid 501, gid 20. The probe reported
zero effective, permitted, inheritable and ambient capabilities. Its
capability bounding set was not dropped, so this is not an exact recreation
of Pinfold's privilege configuration. The observed operations were ordinary
unprivileged file operations.

The fixtures were plain files named `.git`, not real repositories. Every
mutation scenario used a separate directory so an earlier successful
mutation could not contaminate another scenario. A host controller recorded
file contents, existence, inode identity and link counts before and after.
No host-execution payload was used; loss of file integrity already refutes
the proposed enforcement mechanism.

## Policies and controls

All policies began with:

```scheme
(version 1)
(allow default)
```

The selective policy added an absolute-path denial for each fixture's
protected file:

```scheme
(deny file-write* (literal "/private/tmp/.../literal/.git"))
```

The whole-share policy instead added:

```scheme
(deny file-write* (subpath "/private/tmp/.../fixture"))
```

Before creating the VM, the same Swift process attempted ordinary writes
and writes to protected files, including the exact `literal/.git` and
`case/.GIT` paths later reached through the guest. This establishes that
the policy was active in the VM-owning process before VZ obtained the share.

| Arrangement | Native ordinary write | Native protected writes | Guest ordinary write | Guest protected mutations |
|---|---|---|---|---|
| Writable share, allow policy | Succeeded | Succeeded | Succeeded | All succeeded |
| Writable share, selective Seatbelt denial | Succeeded | Denied with EPERM | Succeeded | All succeeded |
| Writable share, whole-share Seatbelt denial | Denied with EPERM | Denied with EPERM | Succeeded | All succeeded |
| VZ read-only share, allow policy | Succeeded | Succeeded | Denied | All denied |

The guest attempted:

- Truncating and writing `.git` and `.GIT`.
- Unlinking either spelling.
- Renaming `.git` away.
- Renaming a prepared replacement over either spelling.
- Writing through a preexisting hardlink.
- Creating a new hardlink to `.git`, then writing through it.

For the selective and whole-share policies, the host observed changed
contents, missing unlinked/renamed paths, changed inodes after replacement,
and the expected increased link count after hardlink creation. These are
host observations, not conclusions drawn only from guest exit codes.

For the read-only-share control, the guest changed none of the fixture
files. Some protected files already contained `native` from the successful
pre-VM native control; those retained that content. Guest errors were
permission denials and all ten guest operations returned failure.

Representative selective-policy output:

```text
NATIVE native/ordinary fd=3 errno=0
NATIVE literal/.git fd=-1 errno=1
NATIVE case/.GIT fd=-1 errno=1
VZ_SHARE_READONLY false
VZ_CONFIG_VALID
VZ_STARTED
GUEST_BOOTED
uid=501(fixture) gid=20(fixture) groups=20(fixture)
CapEff: 0000000000000000
RESULT ordinary 0
RESULT literal 0
RESULT case 0
RESULT unlink 0
RESULT unlinkcase 0
RESULT rename 0
RESULT replacement 0
RESULT replacementcase 0
RESULT prelink 0
RESULT newlink 0
GUEST_PROBE_DONE
VZ_GUEST_STOPPED
```

An initial three-VM run established the failure. A second four-VM run
added native checks on the exact guest target paths and the VZ read-only
control. Both runs agreed. All seven VMs reported normal shutdown. Their
subsecond toy-VM lifetimes are not Pinfold startup benchmarks.

## Scope and consequence

This is a real VM filesystem experiment, not a Pinfold CLI integration
test. It isolates the framework sharing mechanism used by the installed
runtime without changing the runtime's launch services. It does not
identify the private framework process that performs every host request,
prove how sandbox extensions are delegated, or test a custom exporter.

Seatbelt around the existing VM launcher is therefore insufficient for
safe full-worktree editing on the current case-insensitive filesystem.
The next design must enforce restrictions in the actual export service
or change the working-copy boundary. Removing guest Git and adding a
Switchyard commit command still would not protect a writable host pointer.

The native Seatbelt alternative remains a separate architecture: running
the agent directly as a sandboxed Mac process changes the Linux environment
and VM boundary, and retains the previously observed hardlink concern.
It was not implemented or tested here.

The probe source and raw logs were retained in the temporary research
directory `/private/tmp/pinfold-seatbelt-vm`; generated fixture directories,
VM binaries, initramfs and compiler cache were removed after verification.
No production code, security contract, user repository or existing
container was modified.
