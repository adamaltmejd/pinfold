# Project-command review corrections

2026-11-15. Baseline e71c9da, with implementation 7e5eec8. Corrected by
7d3b2cc and 4954dc0. All seven review items identify real code gaps.
The original record's no-departure claim was wrong: fresh-disk
`lost+found` cleanup had been removed outside the plan.

## Changes and cases

1. Trusted init classifies project setup and executable lookup failures
   before CMD as 125. Genuine command exits 125/126/127 pass through;
   public exec lookup remains 127. Linux's missing guest cwd reproduced
   127. Apple instead created the missing workdir and ran CMD. Both now
   refuse it as 125 without the CMD marker, with a plain-directory control.
   External box down reproduced project exit 0, then 125. A separate
   startup case confirms accepted cancellation before any launch or
   terminal record: baseline 0, fixed 125. Host SIGTERM remains 143.
2. Guest cwd walks directory descriptors from `/`, without symlinks,
   through the project root and relative cwd; `fchdir` uses the held FD.
   Guest-planted symlinks to home reproduced the escape. Both external
   and internal symlinks now refuse; a plain subdirectory succeeds.
3. The selected profile's `share` is checked unconditionally, including a
   missing target. The dangling link into PROJECT/missing reproduced an
   allowed launch. It now refuses host-path-in-project before project
   state, and the same project runs with its share linked outside.
4. Authored diagnostic values use the shared escaping helper, including
   captured Git stderr, executable names and updater/crossing paths.
   Guest hook stderr and newline executable/host paths exercise these
   boundaries. Direct command output still contains its literal newline.
   Both updater path branches ran on each host, including successful
   replacement of only a disposable executable copy.
5. Main owns saved termios on both platforms, outside the cancellable exec
   task. This avoids relying on an unjoined raw guard finishing in time.
   The reported raw-terminal symptom did not reproduce in either the
   ordinary or blocked-output baseline sample. That is not evidence the
   unjoined lifetime was safe. Before/fixed cases retain actual stty -g;
   fixed interrupts report sane settings. Linux kernel seccomp refuses
   TCSETS for only the test process descendants: baseline dispatches CMD,
   fixed project refuses 125 and public exec refuses 1 without its marker.
   A real nested trusted init receives dimensions on a pipe; its guest
   TIOCSWINSZ fails and returns 125. Redirected public forced-TTY stdout
   reproduced Apple's failure and now works. Zero host dimensions produce
   no explicit size metadata; unknown native guest dimensions may be 0x0.
   Linux applies initial size once in open_pty. A later resize ioctl error
   reports FAILED through the existing transport, without guest signalling.
6. Only empty formatter lost+found on fresh disks is removed. Both hosts
   reproduced its presence; both fixed fresh disks report its absence.
7. Delegate task/follow-up examples use -- pi, status 125 and actual
   diagnostic prefixes. Their real bundled Pi version invocations succeed
   on Mac. The small Linux image has no Pi; its literal argv cases pass.
   Push/fetch usage now says "its box". Skill frontmatter is unchanged.

## Reproduction and evidence

Use a fresh short root per invocation. The scripts select isolated config
and state, stamp maintenance, disable update checks and request 2 CPUs/1G.

```sh
python3 review-cases.py "$BIN" "$IMAGE" baseline_or_fixed
python3 startup-control.py "$BIN" "$IMAGE" baseline_or_fixed
python3 tty-review.py "$BIN" "$IMAGE" baseline_or_fixed
python3 update-case.py "$LOWER_VERSION_BIN"
```

Mac used the existing localhost/pinfold/profile-default:latest. Linux
reused the active trial lease, its static x64 musl binary and the existing
localhost/pinfold/image-probe-commands:latest. VM operations used labvm exec
-- bash -lc; long jobs used setsid nohup and retained logs. No real login
or credential store was used. No image, dependency or suite test was added.

[mac.txt](2026-11-15-project-command-review/mac.txt) and
[linux.txt](2026-11-15-project-command-review/linux.txt) retain baseline
excerpts, fixed CLI cases, startup cancellation and affected original
acceptance. Original argv/binary-I/O/status/persistence/concurrency cases
passed again. The original terminal driver also passed initial 36x120,
TERM/COLORTERM precedence and native resize to 41x133. Its derived copy
normalizes only Darwin's PENDIN state when comparing configured termios.
[terminal.txt](2026-11-15-project-command-review/terminal.txt) retains
raw stty settings; the SDK identifies PENDIN as pending-input state.
[update.txt](2026-11-15-project-command-review/update.txt) records real
public-release downloads and verified replacement of the scratch copies.
The fixture changes only package/lock versions to 0.0.0, with no test flag
or alternative updater code path. The installed pinfold is untouched.
[checks.txt](2026-11-15-project-command-review/checks.txt) records builds,
fmt, strict Clippy on both hosts, fixture construction and cleanup.

No finding was skipped as false. The 5a raw-state race remains unobserved
empirically; its guard-lifetime gap is confirmed in code and removed.
Later resize-ioctl failure forwarding was reviewed, not fault-injected;
the initial guest ioctl failure was exercised on both hosts. The suite
remains suspended. No push, PR or release was made. All named disposable
disks were removed; existing images and the active VM lease were retained.
