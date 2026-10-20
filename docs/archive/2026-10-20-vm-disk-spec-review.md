# VM disk spec review (#112)

2026-10-20. This is an independent review of #112 with Adam. It records
the decisions and the checks run for them. The design rationale is in
`2026-10-19-vm-disk-design.md`.

## Decisions (Adam)

1. **Crossing is git only, for now.** Host files enter a box once, through
   a disk's `from` copy at creation. After that, only branches cross:
   `push --repo` into the box and `fetch` out. The file `push`, `export`
   and the working-tree part of `pinfold push` are dropped. They raised
   the overwrite and "which copy is newer" questions, and nothing needs
   them yet. They come back as tickets when a workflow does.
2. **No push rights in a box.** GitHub as the transport was considered
   and rejected, because it would need a repo write token in every lane
   box.
3. **Branches, not files, after the start.** A file the operator wants to
   add is committed and sent as a branch. An inbox for uncommitted files
   is a ticket.
4. **The size cap applies box to host only.** `fetch` refuses a bundle
   past 4 GiB. A `from` copy is bounded by the disk's own size, checked
   before copying. The first pi start would otherwise fail on any project
   with a large build directory.
5. **Plain `clean` never removes project disks.** A moved checkout would
   otherwise lose unfetched work. They go only with `--unused` or
   `disk rm`.
6. **Landlock ABI 5 (Linux 6.10+) is required**, with no weaker jail on
   older kernels.
7. **Nested user namespaces stay blocked on Linux.** Init sets
   `user.max_user_namespaces` to 0, and row 5 keeps `unshare -U` failing.
   On macOS this is a ticket (see checks).
8. **Editor config in fetched branches is accepted**, as listed under
   "Not protected".
9. **The guest kernel is pinfold's own build** from kernel.org's signed
   source. Firecracker CI's kernel has no publisher digest.
10. **`profile new --from-project` is removed** until an export exists.
11. **Linux arm64 stops shipping** until a KVM host can test it.
12. **pinfold ships an AppArmor profile** that grants only `userns` to its
    own path, instead of asking users to turn Ubuntu's restriction off.
13. **The acceptance gate passes on no lost work.** Timings are recorded
    for the operator to judge.
14. **Row 37 is split.** The VMM jail is its own row, 43.
15. **The guest listeners' peer check is deleted.** It could never run;
    the kernel config's lack of vsock loopback is asserted instead.

## Checks

GitHub hosted runners, a throwaway workflow on 2026-10-20:

| Runner | Kernel | `/dev/kvm` | Landlock ABI | AppArmor userns restriction |
|---|---|---|---|---|
| ubuntu-26.04 | 7.0.0-azure x64 | `root:kvm 0660`; runner not in `kvm`; a udev rule makes it usable | 8 | on |
| ubuntu-24.04 | 6.17.0-azure x64 | same | 7 | on |
| ubuntu-26.04-arm | 7.0.0-azure arm64 | absent | — | — |

With the restriction on, `unshare -Urm` fails writing `uid_map` (EPERM),
not at mount. A copy of `unshare` under a profile
`flags=(unconfined) { userns, }` mounted a tmpfs in a new user and mount
namespace while the restriction stayed on. The same binary at another
path still failed.

Apple `container` 1.5.0 on an M5 Mac:
- `volume create -s 1G` makes a sparse ext4 image: 2.3 MB allocated,
  1008 MB usable in the box.
- A new volume's top is `0:0 755`, so the box user cannot write it. One
  root `chown` persists for later boxes.
- Two boxes attach one volume read-only at once (`ro` in
  `/proc/mounts`). While one box has it attached, any second attachment,
  read-only or not, fails at VM start ("storage device attachment is
  invalid"), so pinfold's own `disk-in-use` check must come first.
- In the box, `/proc/sys` is read-only, and root lacks `CAP_SYS_ADMIN`
  to remount it. `user.max_user_namespaces` cannot be lowered, and
  `unshare -U` succeeds as the box user.
- `--virtualization` exposes no `/dev/kvm` with the default guest kernel,
  so this Mac is not an arm64 Linux KVM host as configured.

## Evidence the prototypes did not cover

From a read of #111's and #114's outputs:
- Only Landlock ABI 6 ran there. ABI 5 is untested.
- No raw output shows PID 1's `CapBnd` being exactly `CAP_SYS_ADMIN`;
  row 5 is the first observation. Nothing shows box work unable to leave
  the work cgroup, and no row asserts it.
- The half-close behaviour was seen once, with a confounded setup. The
  spec keeps only the rule (stdin's end travels as a frame).
- The "about a second" cost of a transient box was boot time only.
- Only one box was ever jailed. Row 43 uses two.
