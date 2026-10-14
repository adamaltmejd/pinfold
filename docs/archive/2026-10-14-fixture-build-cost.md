# Update fixture build cost — 2026-10-14

Issue [#92](https://github.com/adamaltmejd/pinfold/issues/92) records the Linux
x64 suite's five-minute budget miss. The exact-base main run took 313.43 s;
PR #91 at `4c02659b` took 300.38 s. Both passed 33 tests, failed none and
ignored the separate slow gate. These single cache-sensitive CI runs establish
neither a new regression nor a controlled speedup.

G28 and G31 already share one modified-profile production executable through
`OnceLock`. It uses a private source tree and target directory to avoid
replacing the suite's executable. The extra compile is required by their
real-byte and changed-profile observations. Moving the ordinary build outside
the test timer would not remove work. Sharing the main target would weaken
fixture isolation. Neither was selected.

The retained change sets `CARGO_PROFILE_DEV_DEBUG=0` only on the disposable
fixture build. It removes unused debugger information. Production source,
debug assertions, overflow checks, optimization and panic settings remain
as before. The only edited fixture source content remains the full-profile
package marker. The private target directory stays private.

[Cargo's profile documentation](https://doc.rust-lang.org/cargo/reference/profiles.html#debug)
defines debug information separately from debug assertions and overflow
checks. No product default, dependency, runtime control or test assertion
changes.

## Controlled local observations

The final source at `4c02659b` was copied exactly as the fixture copies it,
then only `profile/full/package.json` received `fixtureFullChanged=true`.
Three alternating pairs used fresh private targets, the same warm registry
cache, Rust 1.99.0 and `CARGO_INCREMENTAL=0` as in CI. No other local builds
or runtime suites ran during measurement. The command was the existing
`cargo build -p pinfold --locked`, with debug level 2 or 0.

| Pair | Normal debug info | No debug info |
|---|---:|---:|
| 1, normal first | 11.157 s | 7.824 s |
| 2, no-debug first | 8.482 s | 7.736 s |
| 3, normal first | 8.255 s | 7.921 s |
| Median | 8.482 s | 7.824 s |
| Executable size | 16,948,840 B | 14,266,304 B |

This supports a 0.658 s median local build saving and 2,682,536 fewer bytes
in the one-use fixture. The first baseline was noticeably slower despite
its fresh target matching the other runs. These native Mac observations do
not establish Linux CI performance or a total-suite saving. Raw samples,
logs and the measurement driver are retained in the local build evidence
bundle. All six builds succeeded; their private directories were removed.

## Independent review and test audit

The independent proxy reviewer approved the exact one-setting diff on
`4c02659b`; that reviewer did not author the change or run its gates. Review
confirmed that the real production build and private artifact boundary stay
intact. G28's verified bytes, inode, executable-lock, owner and symlink checks
and G31's external package marker, notice and override checks are unchanged.
Their sabotage comments remain applicable. No new assertion block or test
function is added, and no sabotage rerun is warranted.

Formatting and Clippy with denied warnings passed. The uninterrupted full
Mac suite passed 33 tests, failed none and ignored the separate slow gate
in 231.81 seconds. This is a validation result, not a paired suite-speed
comparison.

The associated PR records the subsequent Linux gate and budget result.
