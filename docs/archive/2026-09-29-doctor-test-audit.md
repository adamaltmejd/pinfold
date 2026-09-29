# Doctor: issues 72 and 73

Candidate: `a10143c17d5031315b8fb206225d23edf6fe0c0c`.

Doctor skips automatic maintenance. After failed runtime checks it skips
runtime-dependent image, config, linger and disk probes. Linux preflight
failures retain the original error and add independent checks for the user
runtime directory, systemd and the cgroup v2 filesystem.

No tun check was added as a box prerequisite: boxes use `--network none`.
Podman's normal networking, including image builds, can have additional
requirements. The original cloud VM was not available for reproduction.

## Test audit

Keep the new `doctor_reports_without_changing_state` test, guarantee 27.
No existing guarantee covered doctor's read-only behavior. The test uses
an old artifact and a fresh state directory, first with the host runtime
available and then absent from PATH. On Linux it names a deliberately
missing runtime directory. No mocks or test-only paths.

Expected values come from the marker bytes, the empty host directory,
the fixture path and the spec's `runtime-dir` token. Sabotages are restoring
maintenance before doctor and omitting independent host checks. The test
failed before the fix because doctor deleted the artifact, and passed
afterward. Independent review passed the four test-audit questions and
found no blocking issues. The module-header wording finding was fixed.

Rust source lines: tests 4,840; binary 7,595 (including build.rs).

## Manual gates

- Formatting and offline Clippy with warnings denied: passed.
- TruffleHog 3.97.9, Git history, `--no-verification --no-update --fail`:
  passed. The Yard image pins 3.97.8; this run used the installed host tool.
- Full combined e2e gate: passed. macOS 27/27 in 111.87 seconds;
  Linux x86_64 27/27 in 129.82 seconds; Linux arm64 27/27 in 104.98 seconds.
- CI: https://github.com/adamaltmejd/pinfold/actions/runs/36622584423.
- The temporary remote queue branch was removed.

No Yard run or landing. Review was independent manual review rather than
the configured Yard model seat. The absent-systemd and non-cgroup2 failure
branches were not exercised on the original failing host; Linux CI compiled
them and exercised the missing-runtime-directory diagnostic.
