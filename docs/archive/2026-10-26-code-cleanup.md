# Pre-release code cleanup, 2026-10-26

The code-cleanup pass with test-audit's sweep, then the spec pass over
ARCHITECTURE.md and README.md, before the release after v0.2.7. The audit
read the tree at f75b6e7. Two bug-fix PRs landed just before it: #200
(fetch has no deadline, closing #198) and #201 (disks per state root,
closing #182 and part of #167). Four PRs landed from the audit (#203,
#204, #205, #208); the spec pass, README and this report are the fifth.

## Numbers

Before is f75b6e7, measured against v0.2.7. After is the spec-pass
branch tip before this report's commit.

| Measure | Before | After |
|---|---:|---:|
| Commits since v0.2.7 | 98 | 104 |
| Tracked `.rs` lines | 25,216 | 24,139 |
| Largest source file, `crates/pinfold/src/cli.rs` | 1,322 | 1,314 |
| Largest `.rs` overall, `crates/e2e/tests/e2e/box_.rs` | 5,291 | 4,531 |
| ARCHITECTURE.md lines | 1,331 | 1,251 |
| README.md lines | 155 | 146 |
| `crates` diff since v0.2.7 | +10,148 / −4,030 | +10,296 / −5,255 |
| Delete/add ratio, `crates` | 0.40 | 0.51 |
| Whole-tree diff since v0.2.7 | +29,923 / −4,392 | +30,202 / −5,762 |

The release is still mostly additions: the Firecracker runtime, disks and
the crossing landed in it. The pass moved the `crates` ratio from 0.40 to
0.51.

## Possible net and landed net

Possible net is the sum of the audit agents' estimates. Landed net is
each PR's merged diff from `gh pr view`, all files.

| Area | Possible | Landed |
|---|---:|---:|
| Code | about −454 | #203: +235 / −388 (−153); #205: +226 / −324 (−98) |
| Tests | not estimated in lines | #204: +381 / −361 (+20); #208: +181 / −1,058 (−877) |
| Spec | −125 | ARCHITECTURE.md −80 over the pass (1,331 to 1,251), −72 of it in the spec pass |
| Docs | −12 | README.md −9 |

Code landed −251 of about −454. The gap is findings dropped on review,
listed below with reasons; `keep_two_images` landed at −5 instead of −12.
#204 added lines because its rewrites name harder scenarios and
sabotages. #208 carries most of the test cut: Adam's row trim.

The spec pass's own diff is +193 / −176. It includes
`2026-10-26-pins-kernel-ci-procedure.md`, which holds the procedure text
moved out of the spec.

## What each PR removed

- **#203, cleanup of cli, core, crossing, proxy and config.** cli: disk
  rm's private refused closure, a second `idle()` call, an always-true
  check, a BTreeSet copy, main's early return for `update`. ownership:
  the state dir's unread `pid` file (and row 8's "misleading live pid
  file" clause), `Claim.state_dir`, hand-rolled request_stop loops,
  `#[serde(default)]` on Record; its own atomic publish, now
  `core::write_atomic`. runs: the legacy `egress/` removal and its spec
  sentence. box: the init-below-/ guard and hand-rolled epoch seconds.
  plan: a second host-directory refusal, `valid_memory`. crossing: attempt
  loops, `KeptStderr`, duplicate path checks and `git -C` builder. disk:
  `mark_clean`. pipe: a second nonblocking error mapping. proxy: its own
  route-target error. dirs: state_root_key's own ancestor walk. pi launch:
  six spelled-out empty fields. pi-durable (#161): the lexical half of the
  host-boundary check and its three gate cases.
- **#205, cleanup of runtimes, init, images and update.** apple: the
  hand-rolled PATH search for `container` (#70). runtime: the hand-sized
  build-log buffer. podman: the unit struct. firecracker jail: its own
  bounding-set loop and `pidfd_open` (now init's), `Vmm::ended`.
  firecracker: `exec_command`'s `--tty`/`--workdir` branches, a copy of
  vsock's `put`, a second lookup in `stat`, the boot socket unlink.
  `image::mkfs_ext4` and the init disk's hand-rolled mkfs. init: a second
  relay accept loop and usage string. clean: the single-source
  `keep_two_images`. profile: a second list of built-in names.
- **#204, test audit.** Deleted ten blocks (twins and detectors) in rows
  9, 11, 12, 15, 17, 26, 27, 28, 37 and 40. Rewrote rows 4, 6–12, 15, 16,
  27, 28 and 40–43 to the hardest case, each with its sabotage named.
  Moved e2e lib seams to the one file that uses them. Closes #179.
- **#208, row trim.** Rows 9, 10, 11, 15 and 17 cut to the guarantee a
  caller relies on plus the control it proves; the blocks those rows no
  longer claim go. The untested lines left behind are ticketed (#209–#212).
- **This PR, spec pass.** Below.

## Spec pass

ARCHITECTURE.md, wording only; no control changed meaning.

- Pinned artifacts keeps the pin rule in six lines: what
  `scripts/bump-pins.py` moves, pins move only with a release and only up,
  never rehashed at a version, every download matches its publisher's
  digest, a failure aborts before any pin file is written. The nightly procedure and the guest-kernel recipe went
  to `2026-10-26-pins-kernel-ci-procedure.md`; `guest-kernel/` has no
  README to take them. The spec keeps that vmm.toml pins Firecracker and
  the kernel, both checked against a pinned digest, and the kernel's two
  asserted properties: no vsock loopback, config at `/proc/config.gz`.
- Experimental durable caller: launch, pinning and gate rules went to
  `examples/pi-durable/README.md`. What guarantee 35 asserts stays, and so
  do the prototype's dependency names.
- Guarantees preamble: only rows 34, 35 and 36 and how they run. The CI
  runner paragraph and the budget sentence (AGENTS.md has it) went to the
  procedure archive.
- Deduped: Lifecycle step 5's privilege drop now lives once, under Always
  applied, with step 5's unique clauses kept word for word; image label
  rules; profile seed and `containerfile` restatements; Git's restatement
  of Crossing; the init build targets (Code has them); list's image
  identity; podman-builds-only; the pre-ready signal sentence;
  `NODE_USE_ENV_PROXY`'s rationale; "the socket carries only proxy
  connections".
- Configuration lists `PINFOLD_NO_UPDATE_CHECK`.
- Open questions: more than one pi box per project (#213), yard-sthlm's
  LXC (#214) and browser previews (#215) became issues. No remaining
  question is answered by the current design. Two duplicate open issues:
  Apple guest user namespaces (#120) and freed disk blocks (#129).

Guarantees reconciled with the tests. Each of rows 1–21, 23–29, 33, 34
and 37–43 has one Rust test named for it, 37 in all (34 is `#[ignore]`d,
43 is Linux-only). Row 35 is the pi-durable gate and row 36
`scripts/slow-proxy.sh`; both exist. Rows 22 and 30–32 are absent by
design. One mismatch fixed: row 28's title now reads like its test,
`host_updates_are_verified_and_atomic`.

README.md: the docs audit's shrinks (intro, update paragraph, one
build-from-source block, rebuild hint, built-in profile copy, profile
lead-in, `/tmp` note). The audit's `--help` "drift" was not a bug (#203).

### Left in the spec

- Threat model's "No privilege" row still names PID 1's `CAP_SYS_ADMIN`.
  The threat model's controls are exact, and the row is their summary.
- Profiles still names `read-documents`, rtk and ponytail per profile.
  The Images table lists tools, not pi extensions, so this is no twin.
- Updates' `running-boxes` sentence stays: #205 dropped the compat walk
  it describes (reason below).
- Untested spec lines, listed, not cut: a profile's own `profile`
  ignored; the profile `pinfold.toml` layer's override order;
  `profile new --builtin`; `box push`/`box fetch --remote`; `config ROOT`;
  `-V` and `-h`; `attach` to a running pi box; `apparmor-profile` (used
  only by CI). `config ROOT` and `--remote` are the likeliest yagni; each
  needs a caller decision first. #211 and #212 hold the lines #208 left.

## Adam's decisions

- Trim rows 9, 10, 11, 15 and 17 this release, after the cleanup PRs
  merge. Done in #208.

## Rejected, with re-admit conditions

From the audit:

- network.rs: check only the dialed address. Loosens the proxy rule
  (refuse a name with any forbidden record). Re-admit: a spec change
  loosening the rule.
- crossing.rs: drop the `@filter` check. Would rest "a filtered bundle
  fails" on git behaviour. Re-admit: an e2e row proving git rejects it on
  every supported host's git.
- init/firecracker.rs: PID 1's mount re-checks. Defense in depth against
  a host bug (#195, #196). Re-admit: a threat-model decision that PID 1
  trusts the BOOT frame.
- init/firecracker.rs: the vsock peer CID check. An exec service control.
  Re-admit: as above.
- init/firecracker.rs: exec without `init exec`. Reverts #197 (the exec
  child is never dumpable). Re-admit: never.
- firecracker/mod.rs: `/proc/self/exe` for `current_exe`. A behaviour
  change, unmeasured. Re-admit: a bug it fixes.
- crossing.rs revisit trigger: not fired (24.04 ships git 2.43).
  Re-admit: 24.04 dropped from supported hosts.
- login.rs revisit trigger: not fired (codex rust-v0.163.0 still has
  `getAuthStatus`).

Dropped in #203:

- `--help` "drift": not a bug. The repro passed `"box prune"` as one argv
  word.
- cli.rs Shutdown exit code vs pi launch: not twins (1 vs the box's
  status).
- box.rs `down`'s first lock probe: kept. Without it a concurrent `up`
  can take the lock and `down` exits 0 having removed nothing.
- disk.rs `fs::exists` for markers: kept as is. It follows symlinks, so a
  dangling marker would read absent (fails open).
- proxy.rs `log.failed()` in `serve`: not a twin of handle's. It ends the
  accept loop after an audit-log failure; a fail-closed path.
- pipe.rs shared nonblocking retry loop: tried and reverted. It saved one
  line, added an abstraction and changed the login helper's errors.

Dropped in #205:

- update.rs `running-boxes` walk. A test asserts it, and its scenario (a
  live owner running another binary in the same state root) is one the
  executable lock cannot catch. Re-admit: remove that test block first;
  then the walk, `clean::owner_alive` and the spec sentence go together
  as a caller change.
- `keep_two_images` at −12: would make every build trim every source.
  Landed at −5.
- init.rs:L184: misread; the line is the copy-stream doc.

Not cut in #208: `box exec --tty` and `--workdir`. Removing them is a
binary change (#212).

## Tickets

Opened by this pass: #202, #206, #207, #209, #210, #211, #212, and in the
spec pass #213, #214, #215. None is a module needing rework; they are
bugs seen during the gates, untested spec lines, and moved open
questions.

Closed: #161, #179, #182, #198.

## Suite times

Linux CI e2e suite: 249 s on main before #208, 228 s after. Mac suite
about 140 s (#208: 139.1 s test time).

## Release checklist

Row 35 changed (#161), so the prototype gate must run on all three hosts
before the tag. Then `scripts/bump-pins.py`, the version bump, the Mac
suite and CI, per the code-cleanup skill. Caller changes since v0.2.7 go
in the release notes; this pass changed none.
