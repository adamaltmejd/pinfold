# Route-generated login files: PR 3, blocked during implementation

2026-11-15. Starting source `a99c3be`. Branch `login-files-278`.
The brief requires stopping when the plan cannot preserve the current contract.
The draft is not ready to land. No push, PR, release, dependency or test was added.
Evidence: [manual collectors and retained output](2026-11-15-login-files-from-routes/).

## Blocking initialization conflict

The draft follows the prescribed ordering: prepare generated parents under the
disk claim, as the numeric box user, then start the ordinary box. This conflicts
with the existing strict initial `disk.from` copy when that tree contains `.pi`.

Run from the checkout with its built binary:

```sh
python3 docs/archive/2026-11-15-login-files-from-routes/parents.py
```

[parents.txt](2026-11-15-login-files-from-routes/parents.txt) retains all three cases:

- `copy-control`: a fresh home copied from a host fixture containing
  `.pi/agent/settings.json` becomes ready and reads the copied settings. The
  directories belong to uid 501, the numeric box user.
- `route-control`: a fresh empty home with a Pi login route becomes ready.
  The generated parents belong to the same numeric user.
- `route-copy`: the same copied tree plus the Pi route fails with
  `copy into /home/user: pinfold init receive: File exists (os error 17)`.

This is a regression from combining the two existing contracts, not an agent
inference failure. The record stops here as BRIEF.md directs. No alternative
copy semantics or lifecycle redesign was implemented.

Proposed plan correction: complete initial disk population as the box user
before generated-parent creation and configuration attachment. Keep strict copy
behavior and no-follow traversal. Specify how both runtimes perform this phase
without ordinary box work starting. Do not silently skip existing copied files.

Source review also found that `disk::claim` calls `own_disks` before recording
`UNCLEAN` and attachments. Reused-home preparation now writes there, so those
records must precede it and failed-start cleanup must preserve the right recovery
state. [checks.txt](2026-11-15-login-files-from-routes/checks.txt) retains the code
ordering. A crash in this interval was not exercised.

## Early agent gates

```sh
RUSTC_WRAPPER= ZIG_GLOBAL_CACHE_DIR=/tmp/pf3-zig-global ZIG_LOCAL_CACHE_DIR=/tmp/pf3-zig-local cargo build --locked
python3 docs/archive/2026-11-15-login-files-from-routes/gates.py
```

The gates ran before implementation, against the existing default image. They
used manual native-file mounts, plain host-fixture routes, isolated config/state,
a fresh maintenance stamp and synthetic guest credentials. No real login was read.
[gates.txt](2026-11-15-login-files-from-routes/gates.txt) retains the results:

- Pi 1.5.2 autoloaded only `extensions/pinfold-login/index.ts` and selected its route.
  A stored fixture API key replaced its placeholder without changing the route.
- Claude 2.1.311 managed env overrode both user settings and inherited env.
- Codex 0.169.0 used system config, then user config overrode it, then an explicit
  provider option overrode the user selection.

Every observed request reached the expected fixture host. HTTP 400 establishes
configuration selection only. These were not successful inference/tool exchanges.
Project/explicit precedence beyond these cases remains unverified. The first
collector attempt had an env-command syntax error; the retained rerun corrected it
after removing the first attempt's box and disk.

## Draft changes and checks

Uncommitted source/spec work removes harness declarations, validation, membership,
per-exec env, downloads/mounts, launch-record versions, guest pins and their updater
adapters. CLI/attach/Pi use PATH; PINFOLD_ALLOW remains deliberate. Native renderers
use route metadata and placeholders. Generated destinations, HOME and attachment
counts validate early. The host helper has an explicit accessor and host reporting.
Generic disk-relative parent preparation crosses runtime/BOOT interfaces. Apple
owns fresh roots, then drops privilege before descriptor-relative no-follow parent
creation; Linux uses an unprivileged child. Reused ownership is preserved. The
architecture draft records these contracts. The install-manifest comment was
updated. Existing profile seeds and README were not changed before the stop.

The macOS binary built, including its ARM musl init. Final `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings` and `git diff --check` passed;
see checks.txt. The binary hash identifies the executable used by the collectors.
The source draft subsequently received only import/argument-count lint cleanup
and formatting. Its diff is retained locally at `/tmp/pf3-draft.patch`; its hash
and changed-file summary are in checks.txt. The implementation is not committed.

No profile image was rebuilt or created. Probe boxes and home disks were removed.
Final runtime inventory was empty. Free space was 22 GiB at resumption and 21 GiB
at the stop; every build stayed above 12 GiB. No snapshot was deleted.

Not run after the blocker: Linux/VM work and musl Clippy, successful three-agent
tool exchanges, child-agent inheritance, all Pi providers, interactive/attach/Pi
entrypoints, symlink restart and correction, route rename/removal, full precedence,
attachment 24/25, legacy refusals, helper cancellation/concurrency and boundary
marker scans. Startup with all generated mounts was not measured. The 1.283 s
route-control startup is one Pi mount only and is not that measurement.

Draft caller changes, not landed: remove `harnesses`, `--harness`, launch-record
`harnesses`, guest artifact entries and `/opt/pinfold/<agent>` executables; artifacts
reports the host helper/infrastructure. Routes generate native files; Pi login
requires writable absolute HOME; generated collisions/counts refuse early.
