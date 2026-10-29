# Builder verification validation and remaining latency

Date: 2026-10-29. Operator-coordinated validation of
`617de586b9d7a588e4fb611a438511ce4fe73b25`. This note adds no production
change. Raw receipts were read and independently reviewed locally;
no compilation, test, VM operation or public push ran while preparing it.

## Platform checks and five-build cut

Formatting and locked all-target Clippy passed on Linux and Mac. The
operator reports an initial Mac sccache C-write permission error, followed
by successful command-scoped `RUSTC_WRAPPER=''` Clippy/build, with no
global daemon change. The retained direct Clippy log ends successfully;
that initial environment correction is operator-reported here.

At Ring source `617de58`, the full Mac suite passed **24 tests, 0 failed,
1 ignored**, in **86.22 s**. The budget wrapper recorded **86.875937875 s**,
exit 0; the separate outer driver measured **86.898779209 s**. Native
executable hashes before/after matched
`b09ea3bf62fc0bf2c5e8354f9f87b6f5b2220f522ee5002adf5d11c13a5b2b48`.

The five-case-build cut was validated before Ring at source `db3fef5`:
both selected Linux tests passed in **234.47 s**, outer **236.330697602 s**,
including shared cold setup. Musl executable hashes before/after matched
`a9e7604cb7d7122d080c6e8f76e9e1281e009dc4bcccd726ccad6fb50c78ce6e`.
Mac G15 passed at that source in 35.02 s, outer **37.719352083 s**, with
matching `8458dd…ea57` invocation hashes. Mac G23 had passed at `dc45ab6`
with its unchanged test body; the later full Mac suite covers both at the
Ring source. The Linux selected-test result is neither a Ring-source
full-suite pass nor evidence that the Ubuntu CI budget passed.

Initial failed fixture and Linux tmpfs-setup attempts remain preserved.
The operator reports the latter failed during default-image conversion
before either selected case, with scratch incorrectly on a 2 GiB tmpfs.
The corrected invocation used disk-backed `TMPDIR`. That setup failure
does not establish a production I/O bug or a suite pass.

## Changed normal CLI

The changed debug-musl executable was
`f9ff3c007b971722b2813d6749cd4a2d634ae756575cd9c22c9f457077e8c9ca`
before and after the Ring CLI observations below. The old control uses
its separately identified executable.

| Sample | Wall time | Outcome |
|---|---:|---|
| Old cached control, executable `a9e7604c…` | 13.121252320 s | Built. |
| Ring cached build | 9.961736723 s | Built; same warm OCI identity. |
| Ring artifact verification report | 2.650726184 s | Verified the pinned cached bootstrap. |
| Ring cold-cache run | 25.498952457 s | Built; operator reports public acquisition included. |
| Same-size one-byte-corrupted installed root | 2.710590111 s | Exit 1, failed JSON log `builder-integrity: checksum mismatch`. |
| Restored-root positive control | 11.554406429 s | Built; same warm OCI identity. |

The retained driver creates a new cold XDG directory with `exist_ok=False`
and checks the corruption case's public-ref map against its prior snapshot.
The operator observed unchanged refs. The bundle retains timings and
stdio, not separate catalog snapshots or downloaded staging bytes; it
cannot independently reconstruct those observations. This is not a new
public-download byte-capture proof.

The corruption changes byte 65,536 without changing file size, then
restores it and verifies the complete root outside Pinfold. The restored
SHA-256 is the pin
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
The successful restored build is the allowed control. This trial covers
installed-root verification, not corrupted OCI input or a malicious
public archive server.

Linux release compilation passed. Its normal cached build took
**9.061333519 s**, exit 0, with matching before/after executable SHA-256
`6c48d341e157d633e3e0ef032748f8ab0c879cbc67a11915d2fef92c8f714ef2`.
Mac release compilation completed in **17.36 s**. The retained local
release artifact hashes to
`b49a5f5d374f73f1ab41b335a56a38f441b4eae1db330e4f8e32d93742f39aeb`;
the build log does not contain a per-invocation before/after hash pair.

These are single descriptive samples, not a universal speedup or a
statistical causal estimate. Cached latency remains about 10 seconds,
so the few-seconds goal is **not met**. The first cold sample is about
25.5 seconds. Full changed-source Ubuntu CI, its unchanged 300-second
budget, numeric approval and runtime admission remain pending.

## Preserved evidence

Root:
`/Users/adam/.codex/worktrees/ed8e/pinfold/.local-evidence/critical-five-latency-2026-10-29/`.
The original failure notes and receipts are retained separately; no earlier
archive record was rewritten for these results.

| File | SHA-256 |
|---|---|
| `ring-changed-cli.tar.gz` | `e3621053dc4f2c506fde87491c5e132865aa8c9d63b5b0bc56897694b01be0d4` |
| `linux-ring-build-receipts.tar.gz` | `5b9adb0c399b2a8d06c6d9c8333db4089e21d7491edaa706a52bfa9c6af6c324` |
| `linux-critical-five-pass.tar.gz` | `a48befe8109ca707587dcffe9d5a3c30e4d7e415887d25265e00349875167626` |
| `changed-cli.py` | `1c77aeecbe5e812ba29d82a399d82a009f94434716516ed8538fc808809c396a` |
| `pinfold-ring-full-mac.log` | `be93be22db7dd022527e1a87a282fdbcfae8caa185ed67b98e647565bd7ba6c0` |
| `pinfold-ring-full-mac.json` | `6193f925fffb29f1c3c45302f82e3c56526f8da7d6054e6618095bb2eb095955` |
