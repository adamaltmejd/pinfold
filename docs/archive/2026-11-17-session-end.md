# Session departure, #322, 2026-11-17

Implementation: `67181f0` on `session-end-322`. BRIEF.md supersedes the ticket's controller
proposal. No new privilege, capability, guest listener, dependency or test.
The Always applied section is byte-for-byte unchanged. No credential used.
Evidence is in `2026-11-17-session-end/`. Outputs are selected and home paths
are normalized to user. The suite remains suspended.

## Mechanism

Linux's existing unprivileged exec service watches the exec socket for peer
closure independently of command stdin and output. Apple project commands use
the same execution frames over native stdin/stdout pipes. The unprivileged
`init session-exec` helper watches its original stdin pipe for hangup, independently
of the internal bridge. Stdin EOF is an explicit frame and keeps control open.
A killed host client closes its private endpoints. No owner-to-guest cancellation
operation is needed. The native-client SIGKILL probe observed guest stdin EOF
(checks.txt). Passing a socket directly as native stdio and the first copy path
held output until EOF; immediate chunk copies through pipes passed the final run.

The helper's own spawned child creates its session and process group. Only its
retained child id selects the group. Departure sends TERM, waits two monotonic
seconds, then sends KILL. The leader remains unreaped through escalation so its
id cannot be reused. Child completion wins a simultaneously ready hangup. Normal
completion does not cancel background children. Output pumps retain background
writers' pipe readers; on Apple that helper stays until those writers finish.
The host retains EXIT while draining later output within one second.

## Commands and observations

On Mac: `RUSTC_WRAPPER= cargo build --locked`, then
`python3 docs/archive/2026-11-17-session-end/cases.py target/debug/pinfold localhost/pinfold/profile-default:latest`.
The script stamps its isolated state root's maintenance file before any pinfold
command, uses the real cache, initializes a fixture Git project and starts a
kept box with 2 CPUs/1G. It keeps session B responsive and checks its generation.

Linux SSH was refused by the sandbox before access. No workaround was attempted.
Used the existing active lease via the homelab-trial-vm skill and `labvm exec`.
The trial was already Debian 13 with the required tools, usable KVM and a prepared
`localhost/pinfold/image-probe-owner-rereview:latest` image. No image build was
needed, so no image-build duration is claimed. Its dirty existing clone was
preserved. Source was archived into `/tmp/pfs322`, then changed files transferred.
Built with `cargo build --locked --target x86_64-unknown-linux-musl`, reusing its
existing target cache. Ran the same script with that musl binary and fixture image.

mac.txt and linux.txt retain every case's result. SIGINT/TERM/HUP returned
130/143/129; SIGKILL had no client exit code. Cooperative commands disappeared
within 0.34 seconds in these final runs. TERM-ignoring commands disappeared after
2.30 seconds on Mac and 2.12 seconds on Linux. B still executed in the same
box after every departure. Closing a controlling PTY ended A; Ctrl-C reached the
guest, and its shell ran another command and then completed successfully.
The harness drains PTY output while waiting for exit. Earlier harness versions
mistook echoed input for readiness or stopped draining too early; those runs
were replaced. Three stranded fixture boxes and their six explicitly named disks
were removed. The final inventory contains only the shared buildkit container.

A background child survived completion and wrote fresh output after its client
exited. Plain stdin EOF left A running. Observed completion races kept 0 and 23.
Public exec departure left its guest command alive. A setsid child survived A's
departure; stop removed the box. The incomplete B client returned 125 on owner
teardown. Scripts wait on readiness, process state and PTY output, with deadlines.

## Inspection, checks and caller changes

Inspection: crossing leases, client status selection and owner admission are
unchanged. The public Apple exec branch and Linux public wait path remain.
Neither independent control-connection failure with a still-live client nor
crossing during departure was separately injected. The closed-connection watch
is exercised by SIGKILL; owner teardown exercised incomplete-client status 125.
No additional security or arbitrary-output-backpressure guarantee is claimed.

checks.txt retains successful Mac fmt/clippy and Linux musl build/clippy output.
No image, shared volume or cache was deleted. All fixture boxes/disks were removed.
No image was created or left. Mac free disk was 61 GiB before builds, 55 GiB at
completion; trial free disk was 25 GiB. The pre-existing trial lease was retained.

Caller changes: departed project sessions now receive group TERM then KILL after
two seconds. Apple project TTYs are created in the guest. Normal completion and
public exec keep their lifetime. Processes leaving the group remain outside the
guarantee; stop ends the box. No box-spec field, JSON line, CLI flag or exit code
changed. README, Process supervision, Lifetime and guarantee 47 state this contract.
