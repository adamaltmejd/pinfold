# Code cleanup before v0.9.1 (2026-11-26)

Whole-tree read at 54fd25e3 (12 commits since v0.9.0: #499, #500, #501),
12 module readers, the spec and user-docs readers, the trace tables and
three test-audit sweeps, judged and landed in PR #502. The v0.9.0 pass
(2026-11-26-code-cleanup.md) ran the same day; its Rejected list was given
to every reader.

## Numbers

| | Before (54fd25e3) | After (#502) |
|---|---|---|
| Rust lines, tracked `crates/**/*.rs` | 39345 | 39094 |
| Largest source file | `pi/session.rs`, 2428 | `pi/session.rs`, 2415 |
| ARCHITECTURE.md lines | 2464 | 2445 |

- The cleanup itself, in `crates/`: +363 / −614 (net −251).
- The release since v0.9.0, in `crates/`: +1358 / −1573, a delete/add ratio of 1.16.
- Readers estimated about −390 code lines possible; −251 landed.
- Written tickets: 0.
- Mac suite: 12 tests, about 65 s.

## Landed (one commit per module)

- e2e: two `finish() == 0` twins of row 47's exit passthrough (clean.rs,
  sessions.rs C); B's stays, as the only check that EOF on an established
  session is not departure.
- pi/session: `redirect_stderr` and the raw `getsid` through nix;
  `Request::target` becomes `addresses`.
- pi: remote.rs's three hand-built Git calls through `crossing::git`;
  the box-name suffix through `random_hex`; `RefusalReason::token`; one
  `SHARE_GUEST` constant for the profile share path.
- core/ownership: `Claim` holds one `StartupControl` instead of its
  fields; box.rs's stopped-startup path is one match.
- plan/config: the `check_paths_outside` wrapper, `load_dir`'s two
  NotFound matches, `ProjectMount::as_str`.
- cli: `image_built`.
- core/disk: one `rename_noreplace` for disk export and host sockets;
  `marked`; crossing's poll set and `git_input`'s redundant pipes; an
  unreachable `path-not-utf8` check.
- runtime: Apple `StderrDrain`, exec_transport's `wait` and relay write,
  and terminal pgrp calls through nix; `refresh_base` as a let-else.
- proxy (mechanical): `read_head`, `DeadlineStream::new`,
  `connect_upstream` and `network::forbidden` inlined. Check order and
  logged reasons unchanged.
- image: `BuildRequest` owns its fields (job.rs's copy goes); four 0600
  opens through `catalog::new_file`; `SELECTED_BYTES` is a const.
- firecracker and sockets: the `*at` wrappers and the helper reap through
  nix; `existing_host`; inspect's match.
- init: session_io's read and write through nix; `millis`; the fills
  filter.
- README: restatements and mechanism sentences (−14).
- Spec pass: layering statements, Profiles detail, restated rules; the
  `clean --idle AGE` format is now stated (trace-table gap); a stale
  "harness pins" phrase and a wrong `PINFOLD_PROJECT` pointer. Guarantees:
  15 rows, 15 tests; no cell changed. Open questions: both still open.

## Review

Codex Sol on the whole PR, with an Opus review of the proxy, sockets,
ownership and raw-I/O commits. Sol found that replacing the preflight
ELF `PT_INTERP` parse with `cfg!(crt-static)` checks the build setting,
not the executable copied as guest init, and that dropping the
`KVM_GET_API_VERSION` probe lets an openable but unusable `/dev/kvm` pass
preflight; both were restored. It also kept one exit assertion (above).

## Adam's

None changed. Three `spec:` findings were judged keep: the Transport
cleanup-authorization protocol (fail-closed cleanup, rows 8 and 15), the
two protected-directory lists (two different checks), and the default
profile's second copy of the 403 line (deliberate, spec Profiles).

## Rejected (re-admit trigger)

- session.rs `spawn_owner` takes `fork_guard` once — fork-adjacent fd
  timing; a dedicated review of the owner spawn.
- disk_export.rs copy loop through `tokio::io::copy` — row 45's cancel and
  reclaim path; a rework of the export copy.
- proxy connect/intercept shared 200 + ClientHello helper — control-path
  ordering for −4; a proxy rework.
- proxy `read_status_line` inlined — keeps a named step.
- artifacts.rs one `extract` over `GroupChild` — changes builder-install
  cancel; the gzip-on-cancel leftover reproduced.
- init `open_pty` through nix pty — CLOEXEC and musl errno on the exec
  path; nix gains a CLOEXEC openpty.
- builder.rs `capabilities()` inlined — privilege-drop code, −4.
- terminal.rs termios through nix `Termios` — nix truncates unknown flag
  bits on restore; nix keeps raw bits.
- preflight ELF parse and KVM probe — see Review; never, while the spec
  names `PT_INTERP` and `kvm`.
- ownership.rs `!fresh` removal moved into the else arm — saves nothing.
- sessions.rs L275-285 same owner and generation after a kill — row 47's
  direct proof of one generation.
- crossing.rs L346 no `core.hooksPath` — a cheap direct check on row 10's
  hook control.
- profile/default/Containerfile bashrc snippet — marks every image
  outdated for −5.
- README `CARGO_TARGET_DIR` example — a how-to that passes the bar.
- Spec "dups" whose survivor states a different condition (bundle sweep,
  Maintenance tail, L115, L626, L1293, L1463, L1565, L1595, L1981,
  L1999, L2124).

`join_changed`'s empty `PINFOLD_PROFILE` was checked through the CLI: an
empty value is set everywhere and refused as a profile name. No bug.
Sol's re-review then asked to strengthen that kept assertion (B's shell
can exit 0 on EOF before a wrong departure signal lands). The block is as
on main; strengthening it is new test work, outside the pass, and not
taken.
