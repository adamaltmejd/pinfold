# Post-commit directory-sync fault component

Source-only fixture, 2026-10-28. It has not been compiled or run by its author.
It changes no binary, production control, dependency or test assertion.
This is a proposed external Linux kernel-error trial, not cancellation proof.

The canonical source, `crates/e2e/tests/e2e/fsync_eio.c`, starts the normal CLI
with inherited stdin, stdout and stderr. It was moved unchanged into the
guarantee 23 candidate; there is no second source copy here.
The child sets no-new-privileges and installs an architecture-checked seccomp
filter for `fsync` only. It transfers the notification listener to its parent
over a Unix socket. All other syscalls retain their normal behavior.

The supervisor compares each notifying thread's FD, through `/proc/TID/fd/FD`,
with the supplied image-store directory's device/inode. For that directory,
it compares `catalog.json` with the catalog inode observed before launch.
An absent initial catalog has inode zero. It injects `EIO` exactly once when
the directory matches and the catalog inode has changed. Every other notified
sync continues in the kernel. Notification IDs are checked before responses;
an invalidated notification is not counted as an injection.

The caller must provide an existing private image-store directory and exclude
other writers for the whole trial. Use the normal private cache's `images`
directory, without a test override. The fixture identifies a syscall target;
it does not parse the catalog or manufacture an expected image identity.
Pinfold's current publisher renames the catalog before syncing that directory.
Image-object and raw-root syncs precede that rename and continue normally.

The supervisor polls its listener and child pidfd against one monotonic
300-second deadline. On completion, failure, SIGINT or SIGTERM it kills the
child's process group and reaps its children. Subreaping and the kernel's
direct-child list also cover descendants that changed process groups.
Reaping after SIGKILL still depends on the kernel completing task termination.

The receipt is a new, exclusive, no-follow file with mode 0600. Its parent
directory must be owned trial storage. No supervisor data goes to CLI stdout.
The JSON receipt records the injection, notification ID, thread/FD, directory
identity, before/after catalog inode, child outcome and supervisor error.
Exit 125 means the fixture failed or did not inject exactly once. Otherwise
the supervisor propagates the CLI's exit code or signal outcome. Inspect the
receipt as well as that exit code.

## Producer and trial

Compile on the owned Linux trial host with its normal C compiler and installed
Linux UAPI headers. No downloaded source or new library is needed:

```sh
cc -std=c11 -O2 -Wall -Wextra -Werror \
  /absolute/path/to/pinfold/crates/e2e/tests/e2e/fsync_eio.c -o /owned/trial/fsync-eio
```

Prepare the normal private cache, including verified pinned artifacts, and an
existing mode-0700 `/owned/trial/cache/pinfold/images` directory. Use an image
name and context owned by this trial. Invoke the unmodified CLI:

```sh
XDG_CACHE_HOME=/owned/trial/cache \
XDG_STATE_HOME=/owned/trial/state \
XDG_CONFIG_HOME=/owned/trial/config \
  /owned/trial/fsync-eio \
  /owned/trial/cache/pinfold/images /owned/trial/receipt.json -- \
  /owned/trial/pinfold image build durability \
  --containerfile /owned/trial/Containerfile --context /owned/trial/context \
  > /owned/trial/cli.stdout 2> /owned/trial/cli.stderr
```

The required evidence is `supervisor_errno=0`, `injections=1`, distinct catalog
inodes, successful CLI `built`/exit zero, and the fixture's expected COPY bytes
read through a real box started from the emitted ref. Keep the raw stderr to
review the durability warning; do not assert its prose. A control build without
the supervisor must also succeed. This proves post-commit outcome handling for
an injected directory-sync error, not physical crash durability, cancellation
handling, or the whole guarantee 23 test.

Retain compiler/kernel versions, source and executable hashes, exact command,
receipt, stdout, stderr and box observation in owned durable evidence before
the temporary host is released. No E2E-suite adoption is included here.

Primary interface references:
[Linux seccomp notification documentation](https://docs.kernel.org/userspace-api/seccomp_filter.html),
[Linux seccomp UAPI](https://github.com/torvalds/linux/blob/master/include/uapi/linux/seccomp.h),
and [the kernel notification sample](https://github.com/torvalds/linux/blob/master/samples/seccomp/user-trap.c).
