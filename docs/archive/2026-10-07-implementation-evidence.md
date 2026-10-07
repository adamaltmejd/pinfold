# Implementation evidence

Starting release: `f9cd183`, 0.1.9. Candidate work is on
`codex/pinfold-hardening-profiles`. This record distinguishes released
behavior, candidate checks and outstanding verification.

## Released lifecycle reproduction

Using the original checkout's built CLI, an existing profile image and
isolated temporary XDG directories:

1. Start a box and wait for its `ready` event. SIGSTOP its owner.
2. `box down` exits 0 and deletes the owner's state despite its held lock.
3. Start the same name from a second XDG root. It reports `ready`.
4. SIGCONT the original owner. Its cleanup removes the replacement;
   `box exec` on the replacement exits 3.

A separately created abandoned claim directory with no PID file is
refused as `name-in-use`. These are CLI reproductions, not inferred races.
The scratch driver is `/private/tmp/pinfold-ownership-baseline.py`; its
state is `/private/tmp/pf-own-hsc1zyk6`. All boxes it created were removed.

## Optional image recipes on Apple Container

Apple Container 1.5.0 built each complete candidate recipe under temporary
`pinfold-review-<profile>:20261007` tags. No installed profile tag was moved.
The minimal default build took 31.2 seconds with `--no-cache`.

| Profile | Compressed image bytes reported by runtime |
|---|---:|
| default | 70,946,983 |
| documents | 126,019,661 |
| full | 145,760,005 |
| Existing profile-default image | 239,060,366 |

These are compressed image sizes, not allocated host disk usage. Existing
and candidate images were built at different times; this is not a
controlled benchmark of Debian package changes.

The first documents build failed before execution: Apple's resolver
expanded `FROM bun-${TARGETARCH}` to `bun-`. Supplying
`--build-arg TARGETARCH=arm64` made documents and full builds succeed.
The production build adapter must supply the argument. Full's build log
shows only ARM64 Bun, AnyDoc native and rtk asset stages; the AMD64 asset
stages were not fetched. Scratch recipes and logs are under
`/private/tmp/pinfold-profile-build-check`.

The subsequent Mac suite exercised candidate CLI profile selection,
default/full/default switching and a fresh offline documents build.
Linux stage behavior requires the separate Linux suites below.

## Input boundary reproduction

The released CLI accepted `XDG_STATE_HOME=<project>/host-state` for
`pinfold allow` and wrote a trust record inside the writable project.
The input worker ran this with the runtime excluded from PATH.

Earlier isolated CLI checks reproduced acceptance of unknown nested env
keys, ignored non-UTF-8 overrides, a long project basename exceeding the
state filename limit, and doctor extracting embedded profile resources.

## Integrated verification

The ordinary Mac suite passed 33 tests, with the slow guarantee 34 test
ignored, in 235.22 seconds on Apple Container 1.5.0. The run used exclusive
runtime ownership and `caffeinate -i`. Formatting and strict all-target
Clippy passed. A separate real CLI check preserved a non-UTF-8, multiline
`Env::From` value byte-for-byte inside the guest.

An earlier full run had 24 passes and nine failures. The Mac slept during
that run; download and runtime failures coincided with sleep/wake events.
It also exposed three actual defects: a missing test label filter, a
non-UTF-8 filename scenario unsupported by APFS, and an Apple container
name longer than 63 bytes. The filter was supplied, the filename scenario
was restricted to Linux, and the cosmetic project prefix was bounded to
32 bytes. The passing run includes these corrections.

Sol 6.1 implementers and separate Sol 6.1 reviewers checked the production
changes and assertion blocks. Review also corrected test supervision:
nonblocking output reads, bounded cleanup/reaping, and cleanup restricted
to the observed owner and generation. Completed refusal evidence remains
available to assertions instead of being removed by the helper.

The standalone pi-durable fixture passed against the candidate binary and
an actual Apple box. It checked host-authority overlap refusals, boxed
commands, an exclusive checkpoint writer, a crash after mutation but
before tool-result commit, recovery without unsafe replay, and whole-box
cancellation including a background child. The dependency typecheck also
passed. This is evidence for the documented single-tool prototype, not a
complete `ExecutionEnv`, exactly-once shell execution or a Linux run.

The separate guarantee 34 backpressure gate passed in 313.60 seconds. Its
host fixture observed peer closure at the production deadline while the
non-reading guest remained alive. The first attempt was interrupted and
recorded no verdict; its verified orphan box was removed before rerunning.

The final caller-path additions passed targeted real Mac checks: guarantee
9 in 30.61 seconds, guarantee 13 in 19.75 seconds, and guarantee 23 in
31.40 seconds. They cover explicit workdir/TTY, attach selection with two
live boxes, and caller-controlled cache bypass. Guarantee 18's unknown
project/profile TOML and profile-containerfile refusal/control pairs also
passed through the real CLI with no runtime on PATH.

After those additions, the final combined Mac run passed all 33 ordinary
tests in 177.12 seconds, with guarantee 34 ignored as intended. Formatting
and strict all-target Clippy passed on that source. Logs are
`/private/tmp/pinfold-mac-final-20261007.log` and
`/private/tmp/pinfold-mac-slow-20261007-v2.log`.

The published pull request's checks record both Linux architecture
results against the exact candidate commit. Those checks are separate
from this local evidence. No release has been published; release builds
and the release-specific cleanup/spec gate are not claimed.

## Scope and trade-offs

The profile split removes mandatory document and convenience tooling from
the default image. It uses fixed recipe fragments rather than a package
registry, dependency scheduler or extension security model. Profile
metadata no longer extracts files; profile copying belongs in the
existing profile module. One bounded downloader replaces the artifact
and updater download paths. Stable ownership replaces per-XDG PID-based
decisions. The binary retains its nine direct production dependencies.

This is primarily hardening, not a net source deletion. Before final
documentation-only reconciliation, production Rust grows from 7,930 to
9,001 lines; E2E Rust from 5,839 to 7,322. The largest production file is
`cli.rs`, 1,130 lines. The architecture document is 921 lines. The added
ownership protocol, cancellation and failure handling account for much
of the production growth. Image sizes above are measured; startup speed,
build-log peak RSS and builder disk savings are not.

Against the starting release, production Rust has 637 deleted and 1,708
added lines, a delete/add ratio of 0.373. This implementation opened no
new tickets. It implements the admitted review work and records the
remaining measurement and proof limits here; it is not a release audit.

The slow route test does not establish CONNECT's shared-activity clock.
Static review also exceeds the exact interleavings exercised for mutable
image tags, project cleanup versus pre-box startup, and concurrent Codex
refreshes across XDG roots. Those proof limits do not imply observed
failures. No general runtime-selection controls, async proxy rewrite,
disk quota policy or automatic retry subsystem was added without a
separate demonstrated requirement.
