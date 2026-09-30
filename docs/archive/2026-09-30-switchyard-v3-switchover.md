# Switchyard v3 switchover, 2026-09-30

Replaced the local Yard 0.17.5 binary with Switchyard v3's Yard 0.0.2
release. The aarch64 macOS binary matched the release's SHA256SUMS.
The repository has moved from adamaltmejd/switchyard-v3 to
adamaltmejd/switchyard.

Stopped two orphaned v2 daemons whose checkouts had been deleted. Removed
the obsolete v2 configuration and empty runtime files. Moved historical
v2 daily-test logs, nightly marker and watchdog state to
`~/.local/state/switchyard-v2-archive/2026-09-30/`; automatic approval review
refused their deletion. Migrated supported credentials privately to
`~/.config/yard/operator.env`, mode 0600. Installed the v3 launchd service.
Pinfold is its only registered project.

The default implementer is Pi with `deepseek-v4.1-flash` through
`opencode-go`, effort `high`. Review uses Codex with `gpt-6.1-sol`, effort
`medium`. Heavy and planning workflows retain the Claude login.
Automatic approval and two lanes retain the prior project settings.
The protected paths include both the prior gate/spec paths and v3's
staged guidance paths.

The worker image starts from pinfold's default profile. It carries Rust
1.98.1, rustfmt, clippy, the pinned existing trufflehog and an offline
Cargo cache. Candidate gates retain fmt, clippy and history secret scanning.
The landing host gate retains the macOS and both Linux CI suites.
Y-74 remains saved in the 2026-09-29 handoff; no ticket was started.

## Verification

The service responds, the committed configuration is synced into canonical,
and yard doctor accepts it. The worker image built. A real pinfold box
with no egress passed the pinned Pi version check, fmt, offline clippy and
history secret scan. Host fmt and clippy passed too.

Both selected models completed a shell-task smoke check through pinfold's
real credential routes at the configured effort levels. Neither smoke box
mounted host files or the repository. This verifies model access, not a
complete Switchyard implementation/review/landing attempt.

Both Linux CI architectures passed at commit 63f6496:
https://github.com/adamaltmejd/pinfold/actions/runs/36724577214
The temporary CI branch was removed. The first macOS suite had 23 passes
and eight failures while the host ran out of disk space. Removing disposable
builder cache and stale images recovered about 42 GiB; the default profile
then rebuilt successfully.

The macOS rerun passed all 31 tests in 143 seconds. Removed the disposable
verification image and builder cache afterward; about 48 GiB remained free.
The project image is built by Yard on demand from canonical's target head;
`yard doctor` reports no resident image until the first execution builds it.
The board remains idle, with no tickets or attention.
