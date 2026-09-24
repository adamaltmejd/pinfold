# pinfold

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is the spec, and it is binding.
`docs/archive/` is dated history, not a spec. This file holds the working
rules for changing the repository.

## Rules

- Implement the spec. A design change updates ARCHITECTURE.md in the same
  commit. Follow-up work is a Yard ticket, not a TODO in code.
- Living docs (README.md, ARCHITECTURE.md, this file) state the current
  contract only, tersely. Evidence, measurements and rationale go to a new
  dated file, `docs/archive/YYYY-MM-DD-topic.md`. Archive files are never
  updated to track the present.
- Build the smallest thing that meets the spec. No speculative abstractions,
  compatibility paths, or speedups that haven't been measured to matter.
  Delete rather than keep.
- The controls in ARCHITECTURE.md (Always applied, the proxy rules, the
  threat model's controls, trust) are exact. Weakening one is a spec change, never an
  implementation detail.
- Security lives in the pinfold binary. Nothing in an image, the profile or
  a pi extension is security-relevant.
- No secret value in argv, logs, or any file pinfold writes.
- Dependencies are the ones ARCHITECTURE.md lists. A new one needs a reason
  in its commit message.
- Plain prose, short sentences, in docs and comments. Comment only
  non-obvious intent, footguns, issue links and revisit triggers.
- A ticket is admitted for a new guarantee, a bug reproduced through
  the CLI, or a consolidation of one module that changes no test
  assertion and lands net-negative in lines. A proposal born in a lane
  is rejected unless it names the guarantee or bug it serves.
- Before each release: one heavy consolidation ticket per module that
  grew during the release, then the operator's spec pass over
  ARCHITECTURE.md and README.md (cut restatement and rationale,
  reconcile the guarantees table with the tests, close resolved open
  questions). The pass reports the release's delete/add ratio, the
  largest source file, ARCHITECTURE.md's line count and how many of its
  tickets came from lane proposals, in a dated `docs/archive` file.

## Tests

- **End-to-end only.** No unit tests, mocks, runtime stubs, or test-only code
  paths or flags in the binary. A test builds the real binary, starts real
  boxes, and observes from outside (exit codes, output, host files, the
  egress log) or acts from inside through `pinfold box exec`. The e2e crate
  never imports pinfold's internals; its only seams are a user's: the CLI,
  env vars, `.pinfold.toml`, and the box spec.
- **A fixed list.** One test per guarantee in ARCHITECTURE.md, named for it
  (`box_has_no_route_to_host`, not `test_network_3`). A new test needs a new
  guarantee, or a bug to reproduce through the CLI. Nothing tests argv
  shapes, file layout, help text or log wording.
- **Tests must be able to fail:**
  1. Positive controls. Every "refused" test shows the allowed version
     succeeding in the same box.
  2. Assert the reason, not only the failure: a proxy 403 plus the log entry
     naming why. A timeout or DNS error is not a pass.
  3. Seen failing once. Before merge, the test ran against a deliberate
     sabotage and failed. Its comment names the sabotage.
- **Deterministic.** No sleeps: wait on pinfold's readiness signals. No
  retries: a flaky test is a bug to fix or delete. Fixtures run on the host
  (an HTTP service behind a route, a fake OpenAI-compatible model). The only
  public endpoints are `api.github.com` (allowed) and `example.com`
  (denied).
- **Budget:** the suite runs in under 5 minutes in CI.

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```
