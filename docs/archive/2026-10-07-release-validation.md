# v0.2.0 runtime validation — 2026-10-07

All runtime gates below passed. The completed cleanup and test audit is
recorded in [the release audit](2026-10-07-release-audit.md).

## Validated source

The production source, embedded inputs, Rust tests and durable code are
unchanged since `d2a1383`. Later commits record the audit, fix only the Linux
CONNECT fixture certificates and document the demonstrated Linux durable
coverage. The Mac result is reused for those unchanged inputs; it was not
rerun at the later commit.

## Gates

| Gate | Result | Evidence |
| --- | --- | --- |
| Full Mac E2E | 33 passed, zero failed, one intentional slow-test exclusion; 589.70 s | Local `d2a1383`, Apple Container 1.5.0, macOS 27.0.1 arm64 |
| Linux x64 E2E | 33 passed, zero failed, one intentional slow-test exclusion; 207.66 s | [779620e CI](https://github.com/adamaltmejd/pinfold/actions/runs/37673940446) |
| Linux arm64 E2E | 33 passed, zero failed, one intentional slow-test exclusion; 207.45 s | Same CI run |
| Durable recovery | Passed on Mac, Linux x64 and Linux arm64 | Local Mac fixture and both Linux jobs above |
| Slow routed-response deadline | Passed; 350.27 s including binary rebuild | Local production-deadline gate; unchanged code/assertions |
| Slow CONNECT activity/idle | Passed on Linux x64 and arm64 | [779620e slow proof](https://github.com/adamaltmejd/pinfold/actions/runs/37673934062) |
| Formatting and strict all-target Clippy | Passed | Local and both Linux CI jobs |
| Local release builds | Passed for Mac arm64 and static Linux x64/arm64 | Unchanged production source |

The CONNECT traces show each one-way stream reaching its final marker at
330 seconds. All four peer closures followed the final traffic by
300.011956–300.013068 seconds on x64 and 300.208068–300.208677 seconds on
arm64. The final assertions required two `idle timeout` audit reasons,
live guest holders and successful owner teardown. Proof steps took
655 and 654 seconds respectively, including fixture setup.

Both ordinary Linux suites remain below the five-minute CI budget. The
shared local Mac run exceeds that duration; it is recorded as measured,
without attributing the difference to a particular source of cost.

## Fixture correction and review

The first Linux CONNECT run reached real boxed CONNECT/TLS on both
architectures but failed before timed traffic: Python's strict verifier
rejected the generated CA for missing key usage. Commit `779620e` adds
standard CA and server certificate extensions. It preserves strict guest
verification, fixture-local CA trust, production policy, timeout values and
all traffic/idle assertions. The exact certificate commands passed strict
OpenSSL server-purpose/hostname verification; that check was not treated
as runtime proof.

A fresh independent Sol 6.1 reviewer inspected the post-review changes and
the certificate correction, with no actionable code blockers. All ten
previous GitHub review threads were resolved before this candidate was
pushed. The full runtime evidence is recorded above.

The previous G23 missing build-tail marker remains unexplained because
its failed JSON was not captured. Five isolated CLI and four native runtime
captures retained the marker. The unchanged assertions passed in the final
full Mac and both Linux runs. A bounded failure diagnostic is retained;
no evidence attributes the historical failure to the firewall or G9 fix.

## Final size snapshot

Compared with v0.1.9, this release tree, including this record, adds
7,071 lines and deletes 1,229 across 71 files; delete/add
ratio 0.174. It has 9,043 production Rust source lines and 7,803 E2E
Rust lines. The largest source file is E2E `box_.rs` at 4,820 lines; the
largest production file is `cli.rs` at 1,115 lines. ARCHITECTURE.md has
972 lines. The cleanup pass opened zero written tickets.

## Release boundary

The durable adapter remains a prototype: one sequential boxed shell tool,
whole-box cancellation, no complete ExecutionEnv and no exactly-once shell
execution. Platform validation does not expand that contract.

This record establishes runtime validation. Publication and installation
are subsequent actions and are not asserted here.
