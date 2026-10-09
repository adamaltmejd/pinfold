# Yard candidate test audit

2026-10-09. Operator review of Y-4 and Y-5, with independent Astra review.
This is a candidate audit, not the whole-tree release sweep.

## Dispositions

- Y-5, guarantee 2: extend the existing allowlist test to use `.github.com`.
  The real allowed request to `api.github.com` succeeds; `evilgithub.com`
  must receive a policy refusal before DNS. The supplied hostnames and
  configured suffix establish the expected boundary. The previous exact
  entry did not exercise suffix matching. Comments name exact-only matching
  and dropping the leading dot as distinct sabotages.
- Y-5, guarantee 3: add bracketed IPv6 to the existing literal-refusal test.
  Both the proxy response and externally read egress log must identify the
  literal refusal. The same box retains its allowed CONNECT control.
  IPv4 alone did not exercise bracket removal. No new test function.
- Y-4, guarantee 28: observe the release fixture's check request and version,
  retaining unchanged installed bytes and inode for check-only execution.
  Expected version and request observations come from the host fixture.
- Y-4, guarantee 28: strengthen the existing checksum refusal fixture with
  an incorrect digest under the requested name and the correct digest under
  another name. This retains digest-comparison proof while exercising asset
  selection. An earlier candidate served only the other name; operator
  review sent it back because skipping digest comparison would still fail
  for the missing requested entry. The allowed installation remains the
  positive control, with fixture bytes and host inode identity as expected
  values. Comments name both sabotages.
- Y-4, guarantee 28: replace the untouched sibling symlink assertion with
  the symlink actually used to invoke update. Remove the unused fixture.
  The spec's evidence column now names the invoked command symlink. The
  replacement and verification guarantee is unchanged.

No sabotage was executed. No binary implementation, dependency, test
function, mock runtime or test-only binary path was added by either ticket.

## Landing evidence

Y-5's first landing could not start the Mac suite because the host gate
could not locate Cargo. Both Linux suites passed. Operator configuration
now selects the installed toolchain and its actual Rustup home.

The next Y-5 merged head, `7abf1878cd827c09891f1302a2a03d47b8926d18`,
passed both Linux suites in [CI run 37968768646](https://github.com/adamaltmejd/pinfold/actions/runs/37968768646).
Its Mac run completed with 32 passed, one failed and one ignored in 465.53
seconds. Both changed proxy scenarios passed. Guarantee 22's existing host
Git fixture failed before box startup because it inherited the operator's
commit-signing setting and requested a key passphrase.

Operator commit `44a18cb` disables commit signing only in the Mac gate's
test process. A disposable host Git fixture commit passed with that
environment. Astra reviewed the scope: this does not mask the Git mount
assertions or the explicit global hooks-path fixture. It preserves
file-based Git configuration but replaces inherited environment Git
overrides. The user's Git configuration is unchanged. This setup check is
not a successful end-to-end landing.

Y-5 landed at `2254a6268716d7c41f156f84088fc2d706440730`. Its full Mac
suite passed: 33 passed, one ignored, in 211.96 seconds. Both Linux suites
passed in [CI run 37971076637](https://github.com/adamaltmejd/pinfold/actions/runs/37971076637).
Formatting, Clippy, the secret scan and Astra review also passed.

Y-4's pre-fix landing at `d4b80a4ced1f67e459e32354bb3e6558cef29248`
passed both Linux suites in [CI run 37969754343](https://github.com/adamaltmejd/pinfold/actions/runs/37969754343).
Its Mac updater scenario passed, but the full run failed on the same
fixture-signing setup: 32 passed, one failed, one ignored, in 468.09 seconds.
The candidate was rebased onto the operator fix and reviewed again.

Y-4 landed at `fc8d0678c4a6979319a3ade86094717b19611ec5`, including Y-5.
Its full Mac suite passed: 33 passed, one ignored, in 251.09 seconds. Both
Linux suites passed in [CI run 37971676620](https://github.com/adamaltmejd/pinfold/actions/runs/37971676620).
Formatting, Clippy, the secret scan and Astra review also passed.

At the combined landed head, the e2e crate contains 8,043 lines across nine
Rust/Python source files; the binary crate contains 9,091 lines across 29
Rust source files, including its build script. Counts include blank lines
and comments. Before these candidates the respective counts were 8,009 and
9,091. No test function was added.

Linked worktrees remain refused; this test work does not repair or validate
the separately recorded pre-existing-hardlink issue. Y-10 remains parked.
