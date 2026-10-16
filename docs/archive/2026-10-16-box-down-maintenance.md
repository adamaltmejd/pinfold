# Box down and daily maintenance

2026-10-16. Follow-up discovered while validating issue #100. This change does
not resolve the separate Apple Container lifecycle-lock dependency.

On release `3d49304b9c55a7a8605c870db02f178d7c35c985`, a fresh state directory
makes daily maintenance run before `box down` starts its ten-second owner
request. A full Mac run failed the existing fifteen-second whole-call assertion;
that log did not record the slow phase, so its natural cause remains unproven.

An ordinary stock-runtime comparison did not reproduce that failure: fresh-root
down took 10.171 seconds, and premaintained-root down took 10.008 seconds. Both
returned nonzero while the exact owner remained stopped and retained its state.

A controlled real-CLI probe then paused only the fresh requester's exact
read-only image-list client for 6.008 seconds, with an independent resume
watchdog. Fresh-root down took 16.196 seconds and failed the unchanged bound;
the unpaused premaintained control took 10.010 seconds. This demonstrates the
maintenance-before-owner dependency, not the historical sample's exact cause.
Both cases retained the owner generation, native box and state while stopped.
After resume, the owner completed its queued shutdown normally. Only the
original BuildKit remained; no forced removal or mutation timeout was used.

Commit `3326044e2b0899b2dc0ad848d63681227acf6633` excludes `box down` from the
existing daily-maintenance dispatch and documents the exception. Targeted
orphan cleanup, generation checks, owner deadlines, acknowledgement ordering,
explicit native KILL/plain removal and attached-output draining are unchanged.
The next eligible working command still runs due maintenance.

The built fix completed fresh-root down in 10.012 seconds and the premaintained
control in 10.010 seconds. Ownership/state remained intact during both timeouts,
and normal resumed cleanup removed the owned native box and state. No test
assertion changed. Independent agents reviewed the source, diagnostic scripts
and raw results. Formatting and Clippy passed.

The complete unchanged Mac suite passed on stock Apple Container 1.5.0:
33 passed, zero failed, one separate slow gate ignored. Complete Cargo time was
178.899 seconds; libtest time was 178.48 seconds. Source remained unchanged and
only the original BuildKit remained. The separate five-minute proxy-response
gate was not rerun for this maintenance-dispatch change. Linux CI results are
tracked on the pull request.

Local evidence: `/private/tmp/pinfold-100-2026-10-16/`, directories
`cross-root-down-stock`, `cross-root-down-stock-paused-listing`,
`cross-root-down-fixed`, and `full-suite-maintenance-fix-stock`.
