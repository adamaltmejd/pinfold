# Runtime transport rereview, 2026-11-16

Baseline `7efb7c0`. Fix commits: `2b2a4fd`, `2e43832`, `4c31be7`. All five claims were confirmed by reading that source.
None was rejected. This file appends the new findings and observations;
earlier archive files are unchanged. No suite test or dependency was added.

## 1. Durable VMM identity and startup gate

The old liveness function treated every identity read/parse failure as death.
It also recorded identity only after jail::spawn had allowed Firecracker to
exec. Both parts of the claim were valid.

Before publishing a Linux box claim, preparation now atomically writes and
syncs a not-started identity and its parent directory. A cloned VMM child
sets its parent-death signal and blocks reading the inherited alive pipe,
before entering mappings or mounts. The parent opens its pidfd while the
child is unreaped, writes PID/start-time through a temporary file, syncs it,
renames it and syncs the directory. Only then does it send the release byte.
EOF exits without exec. The existing parent-alive poll remains after the
read, covering owner death before the signal was armed. Post-clone startup
errors kill and reap before returning. Failed ownership handoff also kills
and reaps. The Vmm PID field is deleted; the pidfd remains the signal
authority. Each finite builder phase records identity in its shared claim's
state directory, so its own orphan gate covers the running phase too.

The gate now distinguishes not-started, alive, dead and unknown. A missing,
malformed, noncanonical or unreadable identity is unknown and refuses
cleanup. Process-stat errors also preserve unknown liveness. A zombie,
absent process or different start time proves the recorded VMM has ended.

Recovery after an owner crash:

- Before initial identity/claim publication, no child could have been
  cloned. There is no published claim. A fresh name-lock holder removes the
  unfinished state and old shutdown socket during normal preparation.
- After claim publication, before cloning or durable PID publication, the
  complete not-started identity remains. A child, if any, has not entered
  the jail and cannot exec without the token. Owner death closes its pipe
  and/or kills it. Prune removes the pre-start generation; the name starts.
- During the temporary identity write, the published identity remains
  not-started. A partial temporary file cannot authorize execution. It is
  removed with the retired state; the same recovery applies.
- After PID identity rename, including before directory sync or release,
  cleanup checks that exact PID/start time. The child is still gated until
  the sync completes. Owner death ends it; prune and restart then succeed.
- After release, including after exec, parent-death supervision ends the
  VMM. The durable PID/start time confirms its death before resource removal.

The GDB driver killed real owners before clone, before identity write,
with an empty temporary file, with a complete temporary file before sync,
after durable identity before release, and just after release. Before
release, /proc showed the child still executing pinfold in pipe_read.
Pidfds confirmed all observed children ended before pruning. Each case
pruned and ran a same-name corrected exec/down. A separate started box was
killed, then its identity was removed, had its final newline removed, or
made unreadable. Each prune refused and preserved its record and generation
marker. Restoring that fixture's complete identity allowed prune/restart.

Normal new-binary crashes therefore retain a recoverable identity. External
corruption or a pre-fix generation with unknown identity remains protected;
it needs operator recovery of trustworthy identity/death evidence. This
change deliberately does not infer death from those missing bytes.

## 2. Apple per-box inventory errors

The old boxes() propagated one owner-record error and aborted inventory.
Its fallback skipped valid Apple records with existing runtime containers.
Inventory now reports per-box errors, protects their project state and
continues. Under the name lock, valid fallback records use the same
full-generation-checked orphan removal as ordinary pruning. Other ownership
readers remain strict, and corruption still prevents record-less sweeping.

Two real Apple owners reached ready and were SIGKILLed. Native container
inventory confirmed both actual orphans still existed. The first observed
record was corrupted. Public prune reported it, preserved its container and
marker, and removed the second valid container, generation and state. After
repair, prune removed the remaining orphan. The same name then executed
CORRECTED-CONTROL and tore down on stdin EOF. No unrelated orphan was present.

## 3. Non-creating socket inventory

The claim was valid: leftovers() called the creating host() helper. A checked
non-creating opener now returns an empty inventory when H is absent. The
mutating host() still creates and validates H.

In a private Linux user/mount namespace, /tmp was a fresh root-owned sticky
tmpfs. Real doctor and clean --dry-run each returned 0 and left H absent.
The control disk rm of a nonexistent fixture succeeded and created H mode
0700. No shared host mount or store was changed.

## 4. Process and IPC error classification; CLOEXEC

The claim was valid. Socket allocation, pipe/fork, descriptor setup,
report-read, wait and listen errors now retain ordinary I/O handling.
Directory entry and bind failures still carry runtime-path. The child report
distinguishes fchdir from bind failure.

GDB stopped at the real bind helper's fork syscall and suppressed that
syscall externally. ENOSYS reached the CLI as a generic failed event, without
runtime-path. A normal same-name start/exec/down succeeded. The first observer
incorrectly expected EAGAIN on stderr; the actual generic failure was ENOSYS
in JSON stdout. The observer now records the actual injected syscall error.
No binary hook, fake runtime or timing retry was used.

Linux sockets and pipes now request CLOEXEC atomically with SOCK_CLOEXEC and
pipe2(O_CLOEXEC). The installed Darwin SDK exposes neither interface for
these calls; macOS retains immediate fcntl before fork. Private Apple PTY
creation already uses atomic CLOEXEC and is unchanged.

## 5. Builder BOOT listener

The claim was valid: the direct Tokio pathname bind bypassed the helper.
The BOOT listener now binds through bind_at using the retained checked
directory, then becomes a Tokio listener. Its jail mounts that same retained
directory. The broad socket contract remains unchanged.

A real Linux image build under umask 022 stopped at the BOOT bind boundary.
The observer renamed its socket directory and replaced the old name with a
symlink to a sentinel directory. The helper bound BOOT at 0600 in the
retained directory; the target was unchanged. The observer restored the
name before VMM spawn. The actual RUN build succeeded, and its new image
served BOOT-BUILD-CONTROL through a real box exec followed by down.

## Reproduction, checks and limits

Use the prepared images and isolated-root conventions from the earlier
records. The new directory contains manual drivers and bounded output:
[linux.txt](2026-11-16-runtime-transport-rereview/linux.txt),
[mac.txt](2026-11-16-runtime-transport-rereview/mac.txt),
[checks.txt](2026-11-16-runtime-transport-rereview/checks.txt).

```sh
python3 vmm-gate.py "$BIN" "$IMAGE"
python3 inventory.py "$BIN"
python3 helpers.py fork "$BIN" "$IMAGE"
python3 helpers.py builder "$BIN" "$IMAGE"
python3 apple-prune.py "$BIN" "$IMAGE"
```

GDB barriers and pidfds supplied the Linux crash observations, without
sleeps. The final gate run followed the PIDFD-before-release correction.
Fmt and strict all-target Clippy ran on macOS and Linux musl. No earlier
record was edited. The existing dirty Linux checkout was preserved; the
separate trial checkout was reused and the lease retained.

Only inspected: the pre-publication initial-sentinel crash, a partially
written temporary PID payload, the exact rename-to-directory-sync interval,
EOF without the concurrent parent-death signal, PIDFD/ownership-handoff
failure injection, pipe/wait failure injection, and Darwin's descriptor
setup interval. No power-loss durability trial was performed. The earlier
records' unrelated unverified cases remain unverified. This pass did not
repeat the full Apple terminal suite; it exercised the changed Apple prune
path with real containers and a corrected control.

The new Linux probe image is retained as
localhost/pinfold/image-probe-rereview-boot-28423:latest (plus its immutable
build ref). No existing image was removed or rebuilt. Fixture boxes were
removed; these cases created no project disks. No release, push or PR.

Final free disk: Mac Data 35 GiB; Linux 24 GiB.

Caller changes: unknown Linux VMM liveness refuses cleanup; per-box Apple
record errors no longer block unrelated orphans; inventory stays read-only;
process/IPC failures retain generic handling; builder BOOT starts at 0600.
No CLI flags or JSON schema changed.
