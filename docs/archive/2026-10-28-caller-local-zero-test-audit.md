# Caller local zero-layer fixture revision

Date: 2026-10-28. Test-audit approval review of the bounded revision to
guarantee 23. This revision changes tests and that row's description only.
It has not been built or run. Earlier archive notes remain historical.

1. **Guarantee row.** Row 23 now names a zero-layer local profile's config.
   On Linux, the two existing freshness profile builds use `FROM scratch`
   with `ENV PROFILE_PHASE=one` and `two`. Their project builds and doctor
   checks remain; none needs tools or launches the empty profile. The helper
   returns its ImageCleanup guard, which the caller retains through row 23.
   The caller FROMs that profile's latest, copies the default toolchain's
   filesystem and checks `PROFILE_PHASE=two` before the existing stamp,
   context COPY and ONBUILD instructions. Apple keeps FROM the default base
   in both recipes and keeps its existing shared-lock path. Existing host
   assertions, including freshness, EIO receipts, refs, cache and ONBUILD,
   remain in place.
2. **Named sabotage.** The caller's comment names rejecting a zero-layer
   local profile or dropping its configuration. Profile preparation or the
   RUN check then fails. The existing freshness comment also names using
   the wrong profile identity: changing only the selected profile's config
   must still cause image-outdated, and rebuilding must clear it. The
   post-commit sync-error sabotage and real kernel receipt checks remain.
3. **Prior gap and hardest case.** A populated default filesystem did not
   exercise admission and use of a local image with no layers. The revised
   caller requires the selected profile's metadata and another image's
   executable files at once. Its parent ONBUILD remains deferred, while the
   existing child is the positive trigger control. A named local OCI source
   retains a SourceOp even with no layers: this case does **not** cover an
   external image being optimized to scratch and losing its SourceOp.
   The earlier normal CLI external zero-layer proof remains separate manual
   evidence. No fixture-policy exception or broader endpoint allowance is
   inferred from it.
4. **Outside expected values.** `one` and `two` are literal ENV values in
   the host-written profile fixture. `PROFILE_PHASE=two` is independent of
   Pinfold output. Profile repository naming comes from the documented CLI
   contract and the fixture's own chosen name. The expected freshness warning
   and rebuild behavior come from row 23. Expected ONBUILD and context-marker
   bytes remain the host-written recipe and files. EIO is the Linux errno;
   the external supervisor's private receipt must still show one injection
   and no supervisor error. Existing built/ref/latest comparisons continue
   to test the specified agreement between public outputs.

Linux keeps exclusive runtime ownership through preparation, the first
supervised build, its receipt checks and its built/ref/latest checks. It then
uses `RwLockWriteGuard::downgrade` before the second build. The read-guard
storage is declared before fixtures, so cleanup stays protected at scope
exit. The downgrade has no unguarded interval. Other runtime readers can
proceed for the rest of row 23; later exclusive maintenance still waits.

No build or box was added. The two already present profile builds now supply
the config-only fixture. The added DockerHub `tonistiigi/test:nolayers`
dependency and its `foo=bar` expectation were removed from the test. No new
helper, production API, binary flag, mock, runtime stub or dependency was
introduced by this revision. The C supervisor was not edited here.

The prior candidate's retained `linux-row23-current-2.log` reports a failed
row 23 attempt in **309.25 seconds**, with one EIO injection, child exit 0,
and `supervisor_errno=5`. That is a failed previous attempt, not this
revision's timing or a successful suite-budget result. The narrower lock
and reused local fixture make no unmeasured performance claim. The parent
must run the revised row and measure the final suite's five-minute budget.

Changed source spans are `box_.rs:1461-1467` (guard storage),
`:1492-1517` (sabotage, profile lifetime and caller base), `:1592-1599`
(atomic downgrade), and `pi.rs:1407-1468` (returning the existing guard and
using scratch on Linux). The production lines beside those tests remain
the OCI config/layer admission in `catalog.rs`, resolver receipts in
`builder-root/discover/main.go`, local selection in `job.rs`, and the
existing catalog commit outcome. None was edited by this revision.
Only the two changed Rust files were formatted; no test, build, VM, git
command or sabotage execution was run for this revision.
