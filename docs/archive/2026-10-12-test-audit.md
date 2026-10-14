# Issue 89: bounded box inventory

The caller chose bounded listing with truthful failure reporting. Native
overlay throughput remains an upstream investigation. This change does
not make FUSE mandatory or change `--userns=keep-id`.

## Reproduction

A disposable Debian 13 VM ran rootless Podman 5.4.2, kernel
6.12.107+deb13-cloud-amd64, native overlay on ext4, cgroup v2 with systemd,
and seccomp. Podman reported `Supports shifting: false`.

The host held the real `overlay-images/images.lock` with a POSIX record
write lock. Native `podman ps --all --format json` and current-main
`pinfold box list --label dev.pinfold.owner` both blocked. `/proc/locks`
identified the holder and each waiter on the same device/inode;
the waiting Podman processes reported `fcntl_setlk`. The observer stopped
each probe after two seconds. Both listings succeeded after lock release.

This proves that the image-store lock can stall Pinfold's listing through
the real CLI. It does not identify the lock holders in issue 89's original
concurrent-workload failures.

Podman's storage initialization acquires this lock before applying list
filters. A narrower output format or label filter cannot avoid it.
Cold keep-id mappings can create copied, ownership-adjusted layers while
holding storage locks. These are source findings, not a reproduction of
the original workload's root cause:

- [Podman runtime initialization](https://github.com/containers/podman/blob/v5.4.2/libpod/runtime.go)
- [Image-store initialization](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/images.go)
- [Container creation and mapping](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/store.go)
- [POSIX storage locks](https://github.com/containers/podman/blob/v5.4.2/vendor/github.com/containers/storage/pkg/lockfile/lockfile_unix.go)

## Change

Runtime box inventories have a 20-second deadline. Failure kills the
listing client's process group and reaps the direct child. Stdout remains
buffered until a complete successful inventory, so a timed-out list prints
no inventory. The error carries `runtime-timeout` and exits 1.

`box list` skips daily maintenance. A fresh inventory caller therefore
cannot block on cleanup before the listing deadline applies. Other
working commands continue to trigger maintenance. Runtime mutations,
builds, image inventories and image inspection retain their existing
behavior. Apple inventories share the same helper, including inventory
queries used by stat and builder lookup.

## Test audit

Retain the Linux addition to guarantee 9's lifecycle test,
`crates/e2e/tests/e2e/box_.rs:214-288`. It exercises binary code in
`crates/pinfold/src/core/runtime/mod.rs:95-195`, both runtime inventory
call sites, and the maintenance dispatch in `crates/pinfold/src/main.rs`.

1. Guarantee 9 now names bounded listing and preserved ownership under a
   real storage-lock stall.
2. The comment names three sabotages: restore unbounded listing, restore
   pre-list maintenance, or return a timeout without killing the client.
3. Existing blocks list responsive storage. The new block holds the
   runtime's actual lock and uses a fresh caller where maintenance is due.
4. Exit code, error token and empty stdout come from the spec. Lock
   identity and waiter status come from host metadata and `/proc/locks`.
   The host child supplies the owner PID; a pre-lock runtime query supplies
   the image ID. Listings before and after the lock are positive controls
   on the same live box.

No new test function, runtime stub, test-only flag or dependency was added.
The stalled waiter must be gone before Pinfold returns. Direct-child
reaping is implemented with `wait`; waiter absence alone does not prove it.
An independent read-only review found no actionable correctness issue.

## Validation

- Local formatting, strict all-target Clippy and `git diff --check` passed.
- The disposable native-overlay Linux x64 VM passed formatting and strict
  all-target Clippy. Its complete Rust suite passed 33 tests, with one
  intentional slow-gate exclusion, in 749.46 seconds. This was a cold,
  two-CPU VM run, not a measurement of the five-minute CI budget.
- The Mac Rust suite passed 32 tests and failed guarantee 26, with one
  intentional slow-gate exclusion, in 481.44 seconds. The login-route test
  exceeded its existing 90-second first-event deadline. The lifecycle test
  passed. The startup failure did not report `runtime-timeout`.
- A focused Mac guarantee 26 diagnostic with backtraces and unchanged
  deadlines/assertions passed in 393.74 seconds. Runtime observations
  confirmed the live box was running and the Codex route returned HTTP
  200 before the model completed its tool round trip. This did not
  reproduce the first-event timeout or establish its cause. The failed
  full-suite result remains unresolved; the focused pass does not replace it.
- Linux arm64 and the separate slow gates were not run. This is a local
  implementation validation, not a release certification.
- The disposable VM was released after validation.
