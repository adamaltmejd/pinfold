# Apple base refresh verification

Issue [78](https://github.com/adamaltmejd/pinfold/issues/78) described an
unrestricted base pull. Commit `3aacad4` already restricted Apple base
refresh to the native platform, preserved a literal first-stage platform,
and skipped unresolved platforms. That code shipped in 0.2.0 and 0.2.1.
This record supplies the missing fresh-base CLI verification; it adds no
production change.

## Boundary and fixture

The installed 0.2.1 binary ran against Apple Container 1.5.0 on ARM64 macOS,
with exclusive runtime use and temporary XDG directories. Its SHA-256 was
`5ff08790555426323a073e477859daebb5329c4f7311f8cad30744df1d91ef2a`.
The uncommitted issue 86 candidate was not used.

The fixture used BusyBox's public image registry at the same dependency
acquisition boundary as ordinary profile builds. Boxes had no egress.
The immutable `busybox:1.37.0-musl` index was:

`sha256:5cec3fc171c87218698e85a52af7087de727372aae264a787b8112901a5b0092`

Registry metadata was fetched independently and checked against that
digest. It names eight Linux platforms: AMD64, ARM/v6, ARM/v7, ARM64/v8,
386, ppc64le, riscv64 and s390x. Before the first build, the runtime had
neither this image index nor any of its eight manifest snapshots.

Observers were `container image inspect`, the runtime's snapshot files,
and bytes read through `pinfold box exec`. Snapshot allocation is the sum
of host `st_blocks * 512`, not the sparse filesystem's apparent length.
Manifest identities came from the registry index. Apple keys snapshot
directories by manifest digest.

## Results

The first `pinfold image build` used the immutable base without a platform
option and copied a host marker into its context. It exited 0 in 12.42
seconds. Only ARM64 appeared in the runtime's materialized variants and
snapshots. A real box returned the marker exactly; another returned the
BusyBox ELF header with machine value 183, ARM64.

The second build used `FROM --platform=linux/amd64` for its first stage,
then the existing native default profile as its final stage. It copied
the foreign stage's `/bin/busybox` into the final image, without executing
foreign code. The build exited 0 in 12.86 seconds. A real native box read
the copied ELF header with machine value 62, AMD64. Only the requested
AMD64 variant and snapshot were added; the other six platforms stayed
absent. Both `built.base` values matched the independently fetched index
digest.

| Base snapshot | Before | Native build | Explicit AMD64 build |
|---|---:|---:|---:|
| ARM64/v8 | 0 B | 1,161,711,616 B | 1,161,711,616 B |
| AMD64 | 0 B | 0 B | 1,161,596,928 B |
| Other six Linux platforms | 0 B | 0 B | 0 B |

All three boxes acknowledged successful teardown. Cleanup removed only
the two probe image families and the exact new BusyBox digest reference.
All 20 pre-existing references and their resolved image IDs remained
unchanged. All eight BusyBox snapshots were absent again. The same
previously running `buildkit` remained running with its original start
time; no broad prune or builder deletion ran.

## Limits

This verifies the shipped fix through the CLI, including the explicit
foreign-platform case. It does not reproduce the old unrestricted pull,
measure floating-tag movement, or test unresolved build arguments. The
source still refreshes external references and reads Pinfold-built bases
locally.

Guarantee 23's permanent test currently uses a local Pinfold base and
does not detect a regression in external base refresh. No permanent test
was added here: a repeated fresh-base assertion needs controlled registry
content or removal of shared base references. This bounded manual probe
required an initially absent immutable dependency and removed only its
own references. The coverage gap remains explicit.

Scratch evidence, the driver, complete runtime inventories, owner logs,
and command stdout/stderr are under `/private/tmp/pinfold-issue78/`.

Primary runtime sources:
[ImagePull.swift](https://github.com/apple/container/blob/1.5.0/Sources/ContainerCommands/Image/ImagePull.swift)
passes the selected platform to both pulling and unpacking;
[SnapshotStore.swift](https://github.com/apple/container/blob/1.5.0/Sources/Services/ContainerImagesService/Server/SnapshotStore.swift)
unpacks one selected descriptor, or all descriptors without a platform.
