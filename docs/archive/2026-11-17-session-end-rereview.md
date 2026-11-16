# Session departure re-review, #322, 2026-11-17

Read the three re-review findings against `2f28c5c`, AGENTS.md and RULES.md.
Source commits: `e7bdd6c` fixes queued control/completion delivery; `2b1934f`
escapes guest failures; `865487b` establishes and retains supervision.
No dependency, binary test flag, privilege or guest listener was added. No
suite test was added. Always applied is byte-for-byte unchanged. No workload
credential or login store was used. The briefs remain uncommitted.

## Queued EXIT and stdin credit

Verified by inspection: selection preceded the old pending-frame flush.
Finishing that frame could leave EXIT or stdin credit queued while poll had
no remaining event to wake it. Deadline handling could then disconnect first.
One control-flush routine now finishes the older frame, selects and flushes
queued EXIT/CREDIT, then permits polling. It also runs after post-poll child
observation and stdin consumption, before selecting fresh output. Deadline
checking follows the final nonblocking control attempt.

`slow.py BIN IMAGE [baseline]` uses the real CLI, a kept box and peer B.
The command echoes 1 MiB of stdin, then emits a 16 KiB tail and exits 23. An
idle fork retains both output pipes. The continuous-reader control checks
all 1,064,960 bytes. The slow reader consumes 4 KiB per peer acknowledgement,
then pauses while B releases a FIFO gate and performs bounded CPU work.
There are no timed sleeps. Linux's caller pipe is reduced to 4096 bytes.

Both fixed runs returned 23 with all bytes and 260 peer acknowledgements.
Caller-read pauses measured 1.35s on Mac and 3.10s on Linux. The input exceeds
the credit window, exercising returned stdin credit as well as EXIT.
Both pre-fix runs also returned 23. The reported old timing failure was not
reproduced; the ordering defect is source-verified, not an observed 125 here.
These observations do not establish its former failure frequency.

The Linux state-query control exposed a related send/read race: an outbound
STDIN_END could get EPIPE before the already-sent EXIT was read. The host now
disables sending and drains its read side for up to one second. EXIT retains
priority; EOF without EXIT or deadline expiry still reports connection loss.
The intermediate Broken pipe line and successful final controls are retained.

## Failure diagnostics

Confirmed the raw FAILED conversion and public Linux cli::report path.
Existing terminal_text now escapes that conversion before it becomes a host
diagnostic. The same change covers vsock replies, boot and image-job failure
decoders. Project re-escaping is idempotent. Trusted usage and literal command
stdout/stderr are unchanged. The contract now states this boundary explicitly.

`diagnostics.py BIN IMAGE` forwards real execution. Its host-side fault fixture
replaces a returned reply with FAILED containing ESC color sequences. On Mac
it wraps the actual native client. On Linux it temporarily proxies the fixture
VMM socket, preserving the real CONNECT and command execution, and restores
the original socket before stop. This is labelled fault injection, not a
replacement runtime or a new guest listener.

The pre-fix Linux public exec printed raw ESC bytes and failed the safe-text
assertion. The fixed public exec prints escaped text, exit 1. Public Linux stat
also has real-reply and escaped-failure controls. Apple project execution prints
the injected diagnostic escaped, exit 125. Real true returns 0, and literal
ESC bytes from command stdout survive unchanged on both hosts. Boot/image-job
failure escaping was inspected and compiled, not separately fault-injected.

## Supervision and exceptional cleanup

Session ownership is established before fallible nonblocking/connection setup.
Its guard retains the unreaped Child. Normal completion or ECHILD disarms it.
Returned setup/relay errors first drop relay stdio and the sole PTY master,
then TERM the retained group, wait the full two-second deadline, KILL and reap
the leader. Targets still come only from the spawned Child. The existing
pidfd-open-failure kill/reap rollback remains; early setup ordering is inspected.

`cleanup.py BIN IMAGE` queries the helper's limit in a successful control.
Its fault command then lowers the same-uid helper's RLIMIT_NOFILE to 1, making
poll with six descriptors fail EINVAL. A separate-session restorer observes
the closed stdout reader and restores the old limit before later inspection.
A ignores TERM, HUP and PIPE, so pipe loss alone cannot make this case pass.
The private fixture box deliberately has no peer during this process-wide fault.

Both pre-fix runs returned 125 but left A alive until final stop. Both fixed
runs returned 125 and left A absent/zombie. The actual diagnostics name errno
22; restoration reports events=8 and rc=0. Its errno=29 is stale on success.
The fixed intervals were 2.49s Mac and 2.24s Linux. Ordinary control departure
also succeeds. The previous departure matrix passes after the lifecycle
change, including normal background writers, peer isolation and status priority.

## Environment, evidence and caller changes

Scripts and selected output are in `2026-11-17-session-end-rereview/`.
They stamp isolated XDG state before dispatch, use the real cache and remove
only their own boxes and two explicitly named disks. Mac used the existing
profile-default image. Source built with cargo build --locked; Linux adds
--target x86_64-unknown-linux-musl. Fmt and strict all-target clippy pass on
both platforms. Image recipe, digest, build gate and checks are retained.

Dev-VM SSH was refused by the sandbox. No workaround was attempted. The
homelab-trial-vm skill acquired a fresh Debian 13 trial. Its small Git/Perl/Bash
image built in 53 seconds. Evidence was exported before release. A redundant
post-release export overwrote two temporary log copies; selected Linux cleanup
and check lines were restored from earlier tool outputs and labelled as such.
The confirmed release reports idle. All fixture boxes/disks are removed. No
Mac image was created or removed; only shared buildkit remains running. Mac
free disk was 49 GiB initially and 37 GiB at the last check; Linux had 27 GiB.

Caller changes: queued completion/input credit progresses without an unrelated
wake; a failed input send allows bounded receipt of in-flight completion;
untrusted guest failure text prints escaped; returned helper errors end the
command group instead of leaving it unsupervised. Ordinary public exec departure
and normal background lifetime remain. No CLI flag, box-spec field, JSON-line
shape or exit-code definition changed. No push or PR was made.
