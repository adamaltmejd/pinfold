# Remaining configuration proof gaps

Y-7 planning completed on 2026-10-09. A Sol medium Yard worker reviewed
source at `567ff3fb5bcd1565182e90b5cd6357cd535b7092`. The operator reviewed
the proposed scenarios against the binding spec and existing tests.
No production code, test assertion or security guarantee changed.

## Source findings

Three existing contracts have scenarios the current tests do not exercise:

| Contract | Missing scenario | Existing test to extend if admitted |
|---|---|---|
| Guarantee 16, route precedence | A project TOML route wins over a conflicting profile route while the environment route override is absent. Current live route observations use the environment. | `the_highest_layer_sets_the_allowlist` |
| Guarantee 16, empty allowlists | Explicit project `allow = []` overrides a nonempty profile, and present empty `PINFOLD_ALLOW` overrides a nonempty project. Current live observations use nonempty lists. | `the_highest_layer_sets_the_allowlist` |
| Guarantee 18, project selection | `config ROOT` selects a different project's policy from the current directory without runtime or state effects. Current observations select the current directory. | `a_caller_reads_the_effective_configuration_as_data` |

Relevant mechanisms are `Config::load`, `Layer::from_env`, `Layer::over`,
the pi launch plan's egress configuration, and `cli::config/config_report`.
The source supports the intended behavior. Missing scenario coverage is
not a reproduced product defect.

## Operator CLI observations

The installed Pinfold 0.2.2 binary read disposable project directories with
private XDG state, cache and configuration roots. PATH named an empty
directory, so runtime inventory was unavailable. Project A allowed
`example.com`; project B initially allowed `api.github.com`.

| Invocation from A | Observed effective allowlist |
|---|---|
| `config` | `["example.com"]` |
| `config B` | `["api.github.com"]` |
| `config B` with present empty `PINFOLD_ALLOW` | `[]` |
| `config B` after B's project file became `allow = []` | `[]` |

Every invocation exited successfully and reported `image_built: null`.
The private state, cache and configuration directories remained empty.
The disposable fixture was removed. These observations establish reported
policy selection only. They do not demonstrate new live proxy enforcement
or project-route precedence. No model, box or external service was used.

## Disposition

The worker proposed three parked implementation children, proposals 12,
13 and 14. The operator declined them under AGENTS.md's admission rule:
a new guarantee, a reproduced CLI bug, or a qualifying consolidation.
These proposals add proof scenarios for existing behavior and do not meet
those categories. The source findings and proposed observation boundaries
remain available here and in Yard's proposal history.

Reconsider the corresponding scenario on a concrete CLI/runtime failure
or an approved change to its guarantee. A future live empty-policy test
must observe the proxy's named refusal and a working host-fixture route in
the same box. A route-precedence test must observe the selected host
fixture, not only the reported route map. ROOT expectations must come from
the fixture policy and host filesystem, not from another Pinfold report.

Y-7 is complete as planning. No new test was added or full suite rerun for
this read-only review. The prior Y-4/Y-5 landing evidence remains in
`2026-10-09-test-audit.md`. Y-9 still depends on parked Y-10; no worktree
support or hardlink repair was implemented, and the blocked Linux probe
was not retried.
