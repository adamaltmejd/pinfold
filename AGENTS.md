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
  assertion and names what it removes: a concept, a path, a special
  case, a duplicate. Fewer lines is the usual evidence, not the gate; a
  consolidation that adds an abstraction and removes nothing is
  rejected. A proposal born in a lane is rejected unless it names the
  guarantee or bug it serves.
- Before each release, the `code-cleanup` skill: a whole-tree read for
  yagni, duplication, wrong-altitude fixes and unmeasured cost, landed
  one commit per module, or as a heavy ticket when a module is
  reworked; then the operator's spec pass over ARCHITECTURE.md and
  README.md (cut restatement and rationale, reconcile the guarantees
  table with the tests, close resolved open questions). The pass
  reports the release's delete/add ratio, the largest source file,
  ARCHITECTURE.md's line count and how many of its tickets came from
  lane proposals, in a dated `docs/archive` file.

## Tests

- **End-to-end only.** No unit tests, mocks, runtime stubs, or test-only code
  paths or flags in the binary. A test builds the real binary, starts real
  boxes, and observes from outside (exit codes, output, host files, the
  egress log) or acts from inside through `pinfold box exec`. The e2e crate
  never imports pinfold's internals; its only seams are a user's: the CLI,
  env vars, `.pinfold.toml`, and the box spec.
- **A fixed list.** One test per guarantee in ARCHITECTURE.md, named for it
  (`box_has_no_route_to_host`, not `test_network_3`). A new test needs a new
  guarantee. A bug is a scenario its guarantee was missing: the fix extends
  that test and adds none. A bug no guarantee covers is a missing
  guarantee, so a spec change. Nothing tests argv shapes, file layout, help
  text or log wording; a reason is a spec-named token, and the prose around
  it is never asserted.
- **The hardest case.** A test's scenario is where the guarantee's mechanism
  is most likely to give: a race, a second run, a nested or reordered
  input. Not the first case that passes.
- **Expected values come from outside pinfold:** the spec, the runtime, the
  fixture, the host. Never from pinfold's own output or a copy of its logic,
  unless the guarantee is that two outputs agree.
- **Every assertion block earns its place:** a spec line, an observer
  outside the binary, an expected value from outside pinfold, and no twin
  elsewhere. One that fails the bar is deleted and the guarantee row edited
  to match; rewriting is the exception.
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
