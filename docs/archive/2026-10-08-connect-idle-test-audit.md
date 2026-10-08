# CONNECT idle expiry: issue 85

The released proxy checked its shared activity clock only after a blocking
socket read returned. Linux can round a long `SO_RCVTIMEO` wait into a
coarse timer bucket. The five-minute wait can therefore return after the
test's unchanged 320-second upper bound. Traffic in the other direction
often causes a shorter rearmed wait, which hides the delay.

## Reproduction

The unchanged v0.2.1 binary passed the original proof on both Linux
architectures in [run 37754958500](https://github.com/adamaltmejd/pinfold/actions/runs/37754958500).
Observed idle closure was 300.4131–300.4136 seconds on x64 and
300.4971–300.4977 seconds on arm64. That pass did not resolve the issue.

A disposable Debian VM supplied a real CLI trace on a 250 Hz kernel.
A 300-second TCP read took 312.0430 seconds. A rearmed Unix-socket read
took 306.9012 seconds. The complete proof passed, with idle closure at
301.4775–301.4803 seconds. A separate socket experiment kept every peer
open: twelve phased receive timeouts took 300.9230–315.9232 seconds;
twelve equivalent poll waits took 300.0107–300.1003 seconds. The trace
was copied before releasing the VM. This establishes the mechanism, but
does not reproduce the failed upper bound on that kernel.

[Run 37757718360](https://github.com/adamaltmejd/pinfold/actions/runs/37757718360)
then reproduced the real CLI failure with unchanged v0.2.1 production
code, real TLS and rootless Podman. Its Azure 7.0.0-1012 kernel reports
`CONFIG_HZ=1000` and high-resolution timers. An external socket experiment
measured the kernel's expiry cadence before the fixture chose when to
start its TLS connections. No production timeout or assertion was changed.

- Twelve socket receive waits took 300.6538–330.4221 seconds. Twelve poll
  waits took 300.0058–300.1001 seconds, with peers still open.
- Both one-way tunnels delivered all twelve markers over 330 seconds.
- The upload fixture observed idle closure after **329.8521 seconds**.
  The download endpoints observed closure after **300.0280 seconds**.
- The owner and both guest holders remained alive. The upload assertion
  failed at the original 320-second bound.

The job log retains the socket experiment, timing and failure. Uploading
the root-owned syscall trace failed with `EACCES`; that run has no retained
trace artifact. Its immediate failure snapshot contains only the download
closure reason. The proxy appends the reason after both copy threads join,
so that snapshot does not establish the upload's final audit reason.
The original issue's historical run lacks a trace too. This experiment
reproduces the delayed upload path; it does not reconstruct that run's
exact schedule.

Linux's [timer wheel](https://github.com/torvalds/linux/blob/v7.0/kernel/time/timer.c#L495)
uses 32.768-second buckets for a 300-second wait at 1000 Hz. Socket waits
use [schedule_timeout](https://github.com/torvalds/linux/blob/v7.0/kernel/time/sleep_timeout.c#L58).
Poll uses a high-resolution timeout. The measured comparison and the
controlled real CLI failure agree with this source-level explanation.

## Patch and test audit

`core/proxy.rs:626-704` now polls for the remaining shared idle interval,
rechecking the clock after each expiry or interruption. A nonblocking
receive handles stale readiness without changing blocking writes. EOF
still half-closes the destination; failures interrupt both endpoints.
Write deadlines, ordinary HTTP forwarding and audit selection are unchanged.
The patch adds no dependency or test-only binary path.

Guarantee 36 retains its active upload and download cases and adds four
initially idle TLS tunnels, starting eight seconds apart. All guest
processes report readiness before receiving their scheduled connection
time. Timed input uses an event wait; it is not a readiness sleep or retry.

- `scripts/connect_tunnels_share_activity.py:276-299` establishes the
  readiness barrier and phases. Guest receipt of the fixture's final TLS
  acknowledgement supplies the external idle start time.
- Lines 307-338 observe both endpoint closures against the existing
  five-minute production interval and unchanged scheduling tolerance.
  Lines 339-368 retain live guest holders and require all closure reasons
  to be the spec's `idle timeout` token.
- The existing one-way traffic is the positive control. It must continue
  beyond five minutes before either active tunnel becomes idle.
- Named sabotage: restore expiry based only on coarse socket read
  timeouts. Phased idle tunnels expose long waits that a shorter rearmed
  opposite-direction wait could previously mask. No sabotage was run by
  hand. The failure above used the unchanged released binary.
- The new scenarios extend the existing guarantee test. There is no new
  test, mock, runtime stub, timeout relaxation or assertion deletion.
  Independent source and test review approved the candidate.

The issue-85 diff has 52 added and 39 removed binary lines, beside 51
added and 20 removed test lines. The guarantee table changes one row.

## Host verification before publication

The combined CONNECT and Apple reclamation candidate passed the complete
Mac E2E suite: **33 passed, zero failed, one slow test ignored**, in
604.69 seconds. The earlier Apple-only candidate passed the same suite in
412.60 seconds, and focused G15 passed in 64.40 seconds. These are separate
production inputs, not retries of a flaky check. The ordinary host suite
does not run Linux-only G36 or the separate slow G34 gate.

Formatting, strict all-target Clippy and Python syntax checks passed.
Both Linux suites and the expanded G36 proof remain to be run on the
published candidate. Apple issue 78 has separate real-runtime verification
in `2026-10-08-issue78-verification.md`; its production fix was already
released. Issue 86's focused audit is in `2026-10-08-issue86-test-audit.md`.
