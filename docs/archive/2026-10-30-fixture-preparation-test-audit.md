# Fixture preparation and test audit, 2026-10-30

This bounded fixture consolidation starts from
`d62eea978ca85cd3a0410e7aeae154b071258b77`. Its Ubuntu run 41012802604
passed all applicable functional tests: 26 passed, zero failed, one ignored.
The body took 412.67 seconds and the wrapper 415.967693984 seconds. The
unchanged 300-second budget failed. Bootstrap run 41012802782 passed.
These are prior-head results, not validation of this change.

Guarantee 27's private-cache box needs readiness, not the default toolchain.
Its preparation now builds a real FROM scratch caller image through the CLI.
The copied unprofiled binary, AppArmor refusal, reason observations, profiled
readiness control and earlier unchanged-tree observations are unchanged.
The caller image stays in the test's private cache and needs no shared-image
cleanup.

Guarantee 17 reuses the shared managed default image, then creates an empty
default profile directory in its private config. The documented whole-profile
replacement rule leaves this profile without a recipe while its managed tag
exists. The same profile and box name still refuse with image-source-missing;
the same explicit shared image still launches and answers exec. The dedicated
profile build and its ImageCleanup guard are removed. The shared default image
is never scheduled for removal, including on macOS.

## Candidate audit

| Row | Existing failure proof | Outside expected values | Disposition |
|---|---|---|---|
| 27 | Skipping jail preflight loses the copied binary's runtime/userns-apparmor refusal. The profiled executable must reach ready. Running maintenance or materializing profiles changes the observed trees. | Host AppArmor restriction, host tree snapshots, supplied config and the spec's refusal tokens. | Keep every assertion, fault observer and positive control. Replace only the box image preparation. |
| 17 | Inferring a managed ref without a profile recipe selects the existing tag instead of refusing. | The empty user profile directory, the existing real managed image and the spec's image-source-missing token. | Keep every assertion and same-name explicit-image control. Remove the duplicate profile build and its cleanup guard. |

The rows' existing blocks remain their failure proof; no extra assertion block
or test is added. The hardest relevant cases remain a real AppArmor-restricted
copied executable with its allowed control, and an existing stale managed tag
behind a recipe-less profile. No assertion is deleted or rewritten, and the
guarantees table needs no change. Test source changes are 19 added lines and
16 removed lines; binary source changes are zero.

Read-only accounting before this cut counted 14 Linux image builds. The scratch
build replaces one full default preparation; removing G17's dedicated build
reduces that source count by one. G12's real Pi/toolchain setup, the five agreed
G15/G23 build cases and G44 remain byte-identical. No binary, artifact, pin,
derivation, dependency, cap or budget changes.

Doctor's approximately 103-second and G17's approximately 97-second whole CI
windows include contention and other work. They are not isolated savings
measurements. No speedup or new pass is claimed. The worker performs source
review only, with no build, test, VM, push or publication. The operator owns
formatting, compilation and Mac validation before a push, then Ubuntu CI.
Runtime merge, a CLI release, numeric approval and a budget waiver remain
unapproved.
