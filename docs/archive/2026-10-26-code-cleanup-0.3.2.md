# 2026-10-26 code cleanup before 0.3.2

The `code-cleanup` pass with `test-audit`'s sweep, landed as #234, and
the operator's spec pass. Thirteen read-only audit agents (code modules,
three test files, the spec, user docs, trace tables), then landing
agents and one independent review.

## Numbers

| | before (5f7e4ca) | after (ac8f631) |
|---|---|---|
| release delete/add, whole tree since v0.3.1 | +1228 / −166 | +3229 / −4762 |
| the pass alone (#234) | | +2094 / −4689 |
| the pass alone, `crates` | | +1932 / −3465 |
| largest source file | box_.rs, 5047 | box_.rs, 4337 |
| ARCHITECTURE.md lines | 1328 | 1260 |
| guarantee rows | 39 | 27 |
| Linux CI e2e (`e2e_elapsed_ns`) | 243–266 s | 143.7 s |
| Mac suite, host run | 152.7 s | 107.1 s |
| findings that became written tickets | | 0 |

Possible net from the audit: about −600 code and spec lines plus the
test cuts; landed −2595 net, most of it the prototype (−973) and tests.

## Landed

Code, one commit per module: runtime (`describe`, two one-call
helpers), firecracker and init (capability code shared with init, guest
re-checks the host already makes, `Accepted`, `spawn`), pipe/login/
download/proxy (one nonblocking loop, nix `recv`), clean/image/
artifacts/profile (build-time init hash, `copy_tree`, one tag counter,
the documents `package.json` copy), box/plan (one guest-path check,
one digit parser), disk/crossing/ownership/runs (`absent_ok`, shared
flock and private checks), pi launch (one git runner, one UTF-8 check).

Removals Adam approved: update's `running-boxes` check, the pi-durable
prototype with its gate and CI Bun setup, `box push`, `--remote`,
`box exec --workdir`, `profile new --builtin`, `Record.pid`. The
ClientHello parser became rustls's `Acceptor` (−67 lines).

Tests: rows merged 1+7, 2+3, 23+24, 37→5, 25→27, 33→12, 14→26,
38+40+41, and the pi halves of 6, 16 and 18 into 39; rows 29 and 35
cut. Slow scenarios cut: row 26's stalled-helper deadline, row 15's real
pi download, row 19's install repair, row 9's SIGSTOP owner. The disks
test moved to `runtime.rs` so its exclusive lock no longer drains the
shared phase.

## Adam's list and answers

All approved: the four slow cuts, all row merges, the three removals,
naming the two trust checks (Containerfile inside the project; host
roots incl. profile, `trust/`, `artifacts/`, `share` refused inside it),
row wording fixes, the rustls parser, controls listed once.

The review then restored four approved cuts, each the only proof of a
stated control: the second state root's disks, the read-only attach
after recovery, the twin-name `image rm`, and a path-like disk name.

## Rejected, with what would re-admit them

- Dropping curl's `--max-time`/`--max-filesize` on downloads: a bound at
  a trust boundary. Re-admit if the read loop's bounds get a test.
- `trust.rs` canonicalize: cheap normalization of a trust input.
- Row 26's anthropic `pi -p`: the only proof pi takes the subscription
  placeholder as OAuth.
- One `hold()` for the three box-holding sites: needs a mode flag.
- Row 12 retitle: would rename its test.
- `box_stat` move: moves, removes nothing.

## Accepted gaps

- The rustls parser's cap, malformed and IP-literal paths have no test.
- The two newly named trust checks have no test.
- Reseeding and the second-start `disk-in-use` refusal lost their tests
  with row 13's trim.
- `box down`'s timed-out request keeping ownership lost its test with
  row 9's SIGSTOP cut.
- Row 8's prune-writes-`signal` sabotage can no longer bite, since the
  prune runs from the second root.
