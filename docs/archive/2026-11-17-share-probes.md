# Step 5 share probes

2026-11-17. Source `a9e1ec68a76a2e2f854dd8cb82ad296f57cd0701`.
Evidence only for #279, #280 and #298. No implementation or spec change.
The corrected plan is [share-plan](2026-11-17-share-plan.md).
The historical concurrency, read-only Git, linked-worktree and agent records
remain unchanged: [concurrency](2026-11-12-apple-share-concurrency-probe.md),
[Git mount](2026-11-12-readonly-git-mount-probe.md),
[worktrees](2026-11-12-worktree-readonly-git-probe.md),
[agents](2026-11-14-agents-readonly-git-probe.md).
Issues and all comments were read through GitHub's connector; none was edited.

## Environment and reproduction

macOS 27.2.1, arm64. Apple container client/server 1.7.1.
Host uid:gid 501:20. Host Git 2.56.1; guest Git 2.47.3.
Existing image `localhost/pinfold/profile-default:latest`, id
`b3ed8b3ddc10b0526546fc8da159efed2ada7196166826f0943637d1a088be27`.
The image reports pi 1.5.2, Claude Code 2.1.311 and Codex 0.169.0.
Claude differs from the earlier probe's 2.1.325. No image was rebuilt.

The directory [share-probes](2026-11-17-share-probes/) contains five scripts
and their sanitized output. Run each Python script from this checkout.
`linux.py` instead takes the absolute Linux binary path and runs on Linux.
Build with `RUSTC_WRAPPER= cargo build --locked`.
The unmodified build command first failed because sccache was sandbox-denied.
The build with the wrapper disabled passed. No configuration was changed.
`cargo fmt --check` and
`RUSTC_WRAPPER= cargo clippy --all-targets --locked -- -D warnings` passed.
The Linux binary was cross-built here with
`RUSTC_WRAPPER= cargo zigbuild --locked --target x86_64-unknown-linux-musl --bin pinfold`.

All homes and model endpoints are fixtures. No login store was read.
Instruction endpoints run on guest loopback with no network interface.
Public pinfold uses isolated XDG configuration/state, a fresh maintenance
stamp before its first invocation, the real cache and disabled update checks.
Writable shares do not exist in HEAD, so share probes use native container.
These are not validation of pinfold's future share admission or enforcement.
The public read-only control uses real `pinfold box up`, `box exec` and
`box down`: [public.txt](2026-11-17-share-probes/public.txt), lines 4–22.
It reads the fixture, refuses its write with EROFS, preserves host bytes,
reads Git with an exact exception and confirms the removed box with exit 3.

## 23: aliases and attachments

Observed in [mounts.txt](2026-11-17-share-probes/mounts.txt), lines 1–5:
the writable share is at its canonical host path and `.git` is a separate
read-only attachment at the matching guest path. The root is read-only.
Guest work has host uid:gid, CapBnd zero and NoNewPrivs one.
The kernel's user-namespace limit is zero. Ordinary writes reach the host.
Git reads work in this same box with an exact `safe.directory=$PWD` exception.
Without that exception Git refuses dubious ownership: `.git` reports 0:0,
although the containing share reports 501:20. There is no wildcard exception.
The initial multi-command control's rc 0 belongs to its final file write;
the separate successful Git commands have their own exit codes.

Observed, lines 6–20: write/truncate of `.git` itself fail as a directory.
Append, truncate, unlink, rename and replacement of its protected file fail
with EROFS. Removal, rename and replacement of the mounted directory fail
with EROFS or EBUSY. Remount, unmount and tmpfs mount-over fail for privilege.
`unshare -Urnm` fails with ENOSPC while the namespace limit is zero.
This is a namespace-limit refusal, not host disk exhaustion.

Observed, lines 21–25: following the outward symlink cannot read the
unmounted host fixture. Its host bytes remain intact at line 38.
A pre-existing `git-alias` hardlink changes `.git/protected` despite its
read-only attachment. An `outside-alias` hardlink changes the outside inode.
That is the user's deliberately shared content and the chosen rule allows it.
Writing `.git/link`, a symlink to `../a`, changes the writable working file.
This explains the admission refusal of symlinks beneath protected `.git`.

Observed, lines 26–31: child-before-parent attachment order still protects
`.git`. A later writable attachment at the identical `.git` destination
permits the write. A second writable alias at `/alias` also permits it.
Native attachment order alone does not establish the intended protection.

## Admission cost

`timing.py` contains a no-follow reference scanner, executed as host uid 501.
It collects `(st_dev, st_ino)` for multiply linked protected regular files,
counts protected symlinks and checks outside files against that set.
It prunes top-level `.git` from the second traversal. It reads metadata,
not file contents. It reports facts; refusal follows from a symlink or match.
This is a Python measurement, not production admission performance.

Observed, [timing.txt](2026-11-17-share-probes/timing.txt), lines 2–10:
the 100,001-tracked-file fixture also has 1,000 untracked files in ten folders.
The scan visits 101,111 entries outside `.git`.
Samples are 0.202218, 0.195108 and 0.193822 seconds.
This checkout with built `target/` has 4,860 outside entries and 25
multiply linked protected inodes. It has no matching outside alias.
Samples are 0.038433, 0.018684 and 0.017845 seconds.
Tiny fixtures show no match for an outside hardlink, a protected match for
`protected-alias`, and one protected symlink. The selected admission rule
allows the first and refuses the other two. No blanket hardlink refusal.

## 24: privileged setup

Read from `core/runtime/apple.rs:1130–1141`: the current operation is
`container exec --user 0:0 BOX chmod 666 /var/host-services/ssh-auth.sock`.
It supplies neither an absolute executable nor a fixed cwd or cleared env.
`finish_start` calls it after runtime startup; `core/box.rs:868–879`
reaches it after init readiness. The default image PATH contains system
folders, not the fixture share. Spec env is separately encoded under
`PINFOLD_BOX_ENV_*`; a spec PATH does not automatically become root PATH.

Observed, mounts.txt lines 32–37: the same root executable lookup against a
harmless `/tmp/harmless-fixture` selects system chmod with default PATH.
With the native container's PATH explicitly set to begin at share `bin/`,
it executes the fixture script and prints `SHARE_CHMOD uid=0 cwd=/`.
This proves run-environment lookup influence, not a changed image's behavior.
The actual proxy socket was not used: automatic review rejected that probe.
A harmless target preserved the relevant executable-lookup observation.

Read from `apple.rs:718–747` and `init.rs:440–459`:
disk preparation mounts supplied init and selected disks only. Fresh roots
are owned before identity drops; subsequent fill traversal is unprivileged.
The replacement socket operation needs a fixed operation in supplied static
init, a fixed cwd, a fixed absolute target and direct permission syscall.
It must validate the socket without following a substituted symlink.
No image program, shell, PATH or share traversal is needed for that operation.
Nothing was implemented here.

## 25: native instructions and ordinary Git

Observed in [instructions.txt](2026-11-17-share-probes/instructions.txt),
lines 10–21, from actual fixture request bodies:

| Agent | Native home file delivered | Existing instruction delivered |
|---|---|---|
| pi | `.pi/agent/APPEND_SYSTEM.md` | global `.pi/agent/AGENTS.md` and project instruction |
| Claude Code | `.claude/CLAUDE.md` | project `CLAUDE.md` |
| Codex | `.codex/AGENTS.md` | project `AGENTS.md` |

Generic seeds contain both share and disk guidance. Both markers reach
new agent processes for each mode prompt; seed hashes stay unchanged.
Existing native files are preserved byte-for-byte. When the seed path
already exists, its user marker reaches the model and guidance is absent.
Missing-only seeding therefore does not guarantee guidance in every home.
The probe does not choose a new coexistence policy or alter user files.
It changes the mode prompt, not a production mount-kind lifecycle.

All responses are fixture HTTP 400. Agent exits are 1; this establishes
instruction delivery, not compliance, successful inference or tool use.
Lines 8–9 show ordinary Git refusing dubious ownership and succeeding
with an exact `/project` exception. No `safe.directory=*` was used.
The production seed installation and an actual share-to-disk transition
remain implementation acceptance work.

## 26: matched status timing

Observed in [timing.txt](2026-11-17-share-probes/timing.txt), lines 11–49.
The same generated repository is used on host and guest for both command forms.
It has 100,001 tracked files, one tracked edit and 1,000 untracked files.
Expected output is generated independently: one modified path and ten untracked folders.
Global/system Git config is disabled. Both sides get the exact fixture-path
safe.directory exception through GIT_CONFIG_*; no wildcard or Git wrapper.
The guest has four CPUs and 4 GiB RAM. The host has ten CPUs.
Git versions differ, so this is not a filesystem-only causal comparison.

The earlier agent trace observed Claude's
`git --no-optional-locks status --short --ignore-submodules=dirty` and Codex's
`git status --porcelain --ignore-submodules=dirty`. Pi started without a
status subprocess. Those two actual observed command forms are timed here.
Current-image agent startup subprocesses were not retraced: strace is absent.
In particular, the current Claude version differs from the earlier trace.
The table labels command forms, not a new current-agent startup trace.

All values below are seconds; entries are three samples in execution order.

| Command form | Untracked cache | Host Git | Guest Git |
|---|---|---|---|
| Claude form | off | .099, .102, .100 | 15.188, 14.383, 14.276 |
| Codex form | off | .099, .098, .108 | 10.426, 15.227, 10.897 |
| Claude form | on | .079, .068, .068 | 13.852, 13.736, 13.283 |
| Codex form | on | .069, .070, .069 | 10.641, 13.370, 14.127 |

Cache off removes UNTR with update-index. Cache on enables it and runs host
status before the VM starts. The guest receives that host-prepared index
read-only; it cannot persist untracked-cache changes. No cache is added to pinfold.
Cold means a newly created guest VM with no prior Git status in that VM.
Only the first Claude-form sample for each cache condition meets that definition.
Later samples share that VM. Host caches are not flushed; no OS-cold host or
fully cache-flushed host/guest comparison is claimed. Each Git process is new.
No runtime probe was scheduled alongside this final timing run.

Guest monotonic clock time brackets Git spawn and wait inside Perl.
Host monotonic time separately brackets the whole container exec.
Exec-only true commands take .025–.073 seconds; measured outer-minus-inner
overhead is .045–.088 seconds. Host time brackets the direct Git process.
Admission scanning is timed separately before any VM or status sample.
All 24 measured Git commands return 0 with exactly the independently expected output.
Regular-file path/content hashes beneath protected .git match before and
after each guest command, and across each host-plus-guest cache condition.
This is not a comparison of inode identities, modes or timestamps.
The samples support accepting the measured cost, not an optimization claim.
An earlier attempt used absent Node; another used guest wall-clock time,
which shifted. Neither attempt supplies the retained timing numbers.

## 27: Linux ordering, first stage

The sandbox refused the specified dev-VM SSH connection. No bypass was used.
`labvm status` was idle; a three-hour disposable lease was acquired.
The same-HEAD Linux binary and fixture script ran as uid 1000 on Debian,
kernel 6.12.107+deb13-cloud-amd64, with strace and no runtime on PATH.
The repository's public visibility was verified before artifact transfer.
The first automatic transfer rejection was resolved by that evidence.
The probe's isolated config/state/cache roots were absent before each call.

Observed in [linux.txt](2026-11-17-share-probes/linux.txt), lines 3–38:
malformed JSON and unknown mount `kind` both refuse as `spec`, exit 1,
after creating `state/pinfold/maintenance`. Config/cache inventories contain no entries.
The retained trace opens the maintenance stamp before reading stdin.
A parsed relative mount also refuses as `spec`, after an additional
`readlink(/proc/self/exe)`. Correcting that mount reaches runtime validation
and refuses for KVM permissions. It does not start a Linux box.
The extra startup readlink before maintenance is also retained; only the
second one in parsed cases corresponds to init preparation.

Read from `cli.rs:148–155,187–189`, `core/box.rs:199`,
`core/clean.rs:53–70` and `core/artifacts.rs:401–414`:
maintenance precedes parsing; init preparation precedes structural validation.
Linux init preparation returns current_exe; it does not extract/download init.
Mac init extraction can write cache. This qualifies PLAN item 27.
Recognized-share early refusal remains untested until #279 exists.
The corrected successful Linux runtime control also remains unestablished.
The disposable VM's probe directory was removed and the lease released idle.

## Cleanup and scope

Each script removes its own boxes and fixtures. No image or volume was created.
The final runtime inventory contained only the pre-existing running buildkit.
The existing image was left in place. The service stayed running.
The 100,001-file fixture lived under TMPDIR and was removed after measurement.
Free space was checked before every build and large fixture, always above 12 GiB.
The first check reported 17 GiB; final free disk: 44 GiB.
Only this record, its evidence directory and the corrected plan are committed.
BRIEF.md and PLAN.md remain untracked. No ticket was changed or release cut.

## What the builders must respect

Pre-existing protected-inode aliases bypass the read-only attachment.
An outside hardlink shares its selected content without aliasing protected Git.
Protected symlinks can resolve into writable paths. Later attachments can hide protection.
The native mount flag is guest read-only while the backing filesystem reports rw.
Git ownership checks still apply with matching process uid.
Root executable lookup can select share content through its environment.
Native instruction files load; existing seed paths prevent missing-only guidance.
Status cost and correct output coexist; these measurements establish no stale bound.
Linux maintenance mutates state before today's parser refuses an unsupported field.
No current pinfold writable-share boundary was exercised.
