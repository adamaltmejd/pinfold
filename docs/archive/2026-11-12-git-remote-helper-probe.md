# Git remote helper probe

2026-11-12. Item 2 of "Probes before tickets" in
`2026-11-12-project-box-plan.md`: can a remote helper (`git fetch box`,
`git push box`, URL `pinfold::NAME`) keep today's crossing rules against a
hostile box?

Git 2.56.1, macOS 27.2.1 (26B131), Python 3.15.2 (`logs/versions.log`).
Apple `container` for the two end-to-end fetches, pinfold 0.5.5 built from
`origin/main` at 49b4ac3.

Files in `2026-11-12-git-remote-helper-probe/`:

- `git-remote-pinfold`: the prototype helper (Python). `fetch` and `push`
  capabilities only. The box side sits behind `box_make_bundle` and
  `box_take_bundle`: a second local repository (`PROBE_BOX=local:`), a
  file of hostile bytes (`feed:`), a live box through `pinfold box exec`
  (`exec:`), or a transient box over a stopped box's disk (`transient:`).
- `git-remote-pinfoldconnect`: a ten-line `connect` helper, for comparison.
- `craft.py`: rewrites bundle headers for the hostile cases.
- `run.sh TEMP`: every simulated case, in order, from fresh repositories.
  Run twice; the two `cases.log` were identical apart from random temp
  file names.
- `e2e.sh TEMP BINARY`: the two real-box fetches.
- `scrub.sh TEMP`: replaces temp paths (`TMP`, `SYSTMP`) and the account
  name (`user`) in the logs, run after the drivers.
- `logs/cases.log` (every simulated case), `logs/helper-CASE.log` (the
  helper's argv, `GIT_*` environment and every protocol line), `logs/e2e.log`,
  `logs/service.log`.

Host Git ran with `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`,
and the drivers also unset the `GIT_CONFIG_COUNT`/`KEY`/`VALUE`
variables the calling environment set. Size and branch limits were
scaled by environment variable (`PROBE_LIMIT=2000`, `PROBE_BRANCHES=3`).
The mechanism was tested, not 4 GiB or 4096 branches.

"State" below means a diff of `git count-objects -v`, `git for-each-ref`,
a sorted listing of every file under `.git/objects` plus any
`pinfold-incoming-*` quarantine directory, and `git fsck` output with its
exit code. The diff is taken before and after the command. Each case's
log lines are in `logs/cases.log` under `=== CASE`.

## Answer

**Capability: `fetch` (with `list`), plus `push`.** With `fetch`, the
helper writes the objects and Git updates the refs. So the helper can
keep every check today's fetch makes on the box's bytes:
- the listing and bundle limits
- `bundle verify`
- the filtered-bundle refusal
- `index-pack --fsck-objects` in a quarantine
- the connectivity check
- the reach rule for a listed branch the bundle leaves out
- ref-name checks

A bundle the helper rejects leaves no object and no ref. That held in
every hostile case the helper rejected.

The single ref transaction is the open cost. When Git writes the refs
after the helper returns, a box can make that write fail by renaming
branches, a D/F conflict. Without `--atomic` the update is then partial.
With `--atomic` no ref moves, but the objects have already moved into
the repository. Two helper-side fixes were tried:

- **A preflight.** The helper prepares and aborts the same `update-ref`
  transaction against the quarantine. It caught both kinds of failure
  tried, but it only narrows the window.
- **The helper writing the refs itself.** It runs today's transaction:
  prepare, move the packs, commit, then answer `fetch`. That kept the
  D/F case all-or-nothing, objects included, with or without
  `--atomic`. But Git then failed its own update of every existing ref
  with "is at NEW but expected OLD", exit 1, though the ref was right.
  So it works as a transaction and fails as a user interface.

The helper sees configured refspecs and can refuse them. It cannot see a
command-line refspec. `git fetch box main:other` wrote `refs/heads/other`,
and `git pull box main` moved the host's `main` and its working tree.

`connect` is ruled out: host Git took a malformed commit and a `.git`
tree entry straight into the object store.

## Observed

### 1. Capabilities: who writes objects and refs

- **fetch.** `capabilities` then `list` then a batch of `fetch OID NAME`
  lines (`helper-F0-baseline.log`). The helper made the objects present:
  its quarantined packs were moved in at `fetch`. Git then wrote
  `refs/remotes/box/main` ("`* [new branch]      main       -> box/main`").
  Git then ran its own connectivity check: when a desynchronised helper
  answered `fetch` before writing anything (N-early-blank, checks off),
  Git failed with "`error: pinfold::sim did not send all necessary
  objects`".
- **connect** (C1, `-c fetch.fsckObjects` unset). Host Git spoke the pack
  protocol to `git upload-pack` on the box repository. It wrote the
  objects and refs itself:
  - The malformed commit landed as the loose object
    `./58/a90eb1c08faeefd7bc75eb6baf571937f3a848`.
  - The host's fsck then failed: "`error in commit
    58a90eb1c08faeefd7bc75eb6baf571937f3a848: missingEmail: invalid
    author/committer line - missing email`", `fsck-exit=1`.
  - Git also created `refs/remotes/conn/HEAD`.

  C3: a tree with a `.git` entry landed, and fsck printed "`warning in
  tree 4baa8f401b7d47a8d3a15de3438e5e1c447c0fb4: hasDotgit: contains
  '.git'`". C2, with
  `fetch.fsckObjects=true`: refused, "`fatal: fsck error in packed
  object`", exit=128. That run still left `./pack/tmp_pack_*` in the
  object store (`garbage: 1`). `connect` loses everything pinfold checks
  before Git sees the bytes: the size and branch limits, the listing
  shape, branches only, the filtered-bundle refusal and the quarantine.
  fsck becomes the repository's config, not pinfold's.
- **import.** Not tried; see Inferred.

### 2. Hostile bundles with the `fetch` helper

Each case: `git -C host fetch box`. Each ended with exit=128 and "state:
unchanged (count-objects, for-each-ref, object files, fsck)". The
helper's message:

| Case | Input | Helper's message (log line) |
|---|---|---|
| F1 | commit with no author email | `git index-pack failed (exit 128): error: object 58a90eb1c08faeefd7bc75eb6baf571937f3a848: missingEmail: invalid author/committer line - missing email` |
| F2 | `.gitmodules` with `update = !sh -c "echo F2 >>…marker"` | `gitmodulesUpdate: disallowed submodule update setting`; marker file absent |
| F2b | `.gitmodules` with `url = --upload-pack=echo F2b >>…marker` | `gitmodulesUrl: disallowed submodule url`; marker absent |
| F3 | tree entry `.git/hooks/post-checkout` | `hasDotgit: contains '.git'` |
| F4 | bundle cut from 503 to 251 bytes | `git index-pack failed (exit 128): fatal: early EOF` |
| F5 | bundle over `PROBE_LIMIT=2000` | `the bundle is past 2000 bytes` |
| F6a | 5 listed branches, `PROBE_BRANCHES=3` | `the box has more than 3 branches` |
| F6b | 1 listed branch, 5 bundled heads | `the box has more than 3 branches` |
| F7.1 | listed `refs/heads/a..b` | `the box named a malformed branch: 'a..b'` |
| F7.2 | listed `refs/tags/v1` | `the box's branch listing is malformed` |
| F7.3 | listed `refs/heads/x y` | `malformed branch: 'x y'` |
| F7.4 | listed `refs/heads/x:y` | `malformed branch: 'x:y'` |
| F7.6 | listed `refs/heads/@{1}` | `malformed branch: '@{1}'` |
| F7b | bundle head renamed to `refs/heads/x:y` | `malformed branch: 'x:y'` |
| F8 | listed `ghost` at an object neither side has | `branch 'ghost' names 0123456789abcdef0123456789abcdef01234567, which the bundle does not carry and refs/remotes/box/* does not reach` |
| F8b | bundle header names a head its pack lacks | `git rev-list failed (exit 128): fatal: bad object 0123456789abcdef0123456789abcdef01234567` |
| F9a | header `@filter=blob:none` | `the box sent a filtered bundle` |
| F9b | header `@object-format=sha256` over sha1 heads | `git bundle failed (exit 1): error: unrecognized header: b7036811f28d630ef484116a6df59f77f15c520f refs/heads/main (56)` |
| F9c | header `@hooks=post-checkout` | `git bundle failed (exit 1): error: unknown capability 'hooks=post-checkout'` |
| F9d | prerequisite the host lacks | `git bundle failed (exit 1): ` (`bundle verify -q` printed nothing) |

More cases:
- F7.5: a listed `refs/heads/-x` that the bundle did not carry. It was
  refused by the reach rule, not by its name.
- F7c: a real box branch `-x` passed `check-ref-format`. It landed as
  `refs/remotes/box/-x`, exit=0, the same as today's rule.
- F9e: a prerequisite the host holds, with a 100000-byte subject, was
  accepted. The header reader keeps 8 bytes of each line.
- F0 and F0b: honest fetches. F0b's bundle held only what
  `refs/remotes/box/*` lacked: "`b703681..5296044  main       -> box/main`".
- F9a: the `@filter` header was written by hand with `craft.py`, not
  made by `bundle create`.

### 3. Refs

**(a) Partial failure, default refspec.** In R1, the host held
`refs/remotes/box/x`. The box then deleted `x`, created `x/y` and
advanced `main`. The helper's checks all passed.

- **R1, plain `git fetch box`:** exit=1, "`! [new branch]      x/y        -> box/x/y  (unable to update local ref)`".
  `refs/remotes/box/main` moved to 52960449 anyway. Partial.
- **R1b, `git fetch --atomic box`:** exit=254, "`error: cannot lock ref
  'refs/remotes/box/x/y': 'refs/remotes/box/x' exists`". No ref moved.
  But the pack `pack-29bebf79beec4eccfaf3b63aeb197192d4222244` was now in
  the repository and fsck reported "`dangling commit
  52960449ecdbf8fb51d6fee8b0d0a3ae37f97084`".
- **R1c, the helper's preflight** (`PROBE_PREFLIGHT=1`): exit=128,
  "`ref preflight failed: fatal: prepare: cannot lock ref
  'refs/remotes/box/x/y'`". State unchanged.
- **R2, R2b, R2c: the same three with a stale
  `refs/remotes/box/main.lock`.**
  - R2: exit=1. `refs/remotes/box/other` was created and `main` was not.
  - R2b: exit=253. No ref moved; the pack landed and fsck reported a
    dangling commit.
  - R2c: exit=128, the preflight named the lock. State unchanged.
- **O1 to O6: the helper commits `refs/remotes/box/*` itself**
  (`PROBE_OWNREFS=1`). It runs today's `update_refs` sequence at `fetch`:
  `start`, the updates and `prepare` against the quarantine, then the
  pack move, then `commit`, then its answer to Git.
  - O1, a new ref: exit=0, `refs/remotes/box/main` at b7036811.
  - O2, a fast-forward: exit=1, "`error: cannot lock ref
    'refs/remotes/box/main': is at 52960449ecdbf8fb51d6fee8b0d0a3ae37f97084
    but expected b7036811f28d630ef484116a6df59f77f15c520f`". The ref was
    left at the new value 52960449, the right one.
  - O3, a rewritten box branch: the same, "`is at
    d876e12af719d57ae3e80f47829d2c98ba293e90 but expected
    b7036811f28d630ef484116a6df59f77f15c520f`", exit=1. The ref was at
    d876e12a.
  - O4 (plain) and O5 (`--atomic`), the D/F case: "`own ref transaction
    failed: fatal: prepare: cannot lock ref 'refs/remotes/box/x/y'`",
    exit=128, state unchanged in both.
  - O6, `git fetch box main:other`: exit=0. Git still wrote
    `refs/heads/other`.
- **No setting makes fetch atomic.** `git help -c` lists no
  `fetch.atomic`. Atomicity is the caller's `--atomic` flag. The helper
  advertises no `option` capability, so it is never told whether the
  flag was given.

**(b) Refspecs that write `refs/heads/*`.**

- **R3a, `git fetch box main:other`:** exit=0. It wrote
  `refs/heads/other` at b7036811 as well as `refs/remotes/box/main`. The
  helper saw `fetch b7036811… refs/heads/main` twice and nothing naming a
  destination (`helper-R3a-cli-refspec-to-branch.log`). Besides the
  driver's own variables, its `GIT_*` environment held only `GIT_DIR`,
  `GIT_EDITOR`, `GIT_EXEC_PATH` and `GIT_PREFIX`.
- **R3b, `git fetch box main:main`:** Git refused on its own. "`fatal:
  refusing to fetch into branch 'refs/heads/main' checked out at
  'TMP/p/host'`", exit=128. Git had sent `capabilities` and `list`,
  then closed the helper's stdin without a `fetch`.
- **R3c, `git pull --no-rebase box main`:** exit=0. "`Fast-forward`",
  and host `main` went from 0be1ac18 to b7036811. The only trace in the
  helper's environment was `GIT_REFLOG_ACTION=pull --no-rebase box
  main`.
- **R3d, configured `remote.box.fetch=+refs/heads/*:refs/heads/*`:**
  refused by the helper, which read `git config --get-all
  remote.box.fetch`. "`remote.box.fetch writes outside refs/remotes/box/:
  +refs/heads/*:refs/heads/*`", exit=128, state unchanged.
- **R3e, the same setting given with `git -c`:** seen too, through
  `GIT_CONFIG_PARAMETERS`, and refused the same way.
- **R3f, `git fetch pinfold::sim main:other`:** the helper got
  `argv ['pinfold::sim', 'sim']`. It refused, since the first argument is
  not a plain remote name. "`fatal: remote helper 'pinfold' aborted
  session`", exit=128. So a fetch with no configured remote never runs.
- **R3g, control:** the host's own `reference-transaction` hook ran on
  `git fetch box` (`preparing`, `prepared`, `committed` in the marker).
  The repository's hooks apply as to any fetch.

### 4. What reaches host Git from the box

For each case the listing carried one extra line. Each was run with the
helper's checks off (`PROBE_CHECKS=off`, the box's listing passed to Git
verbatim) and on. `git config --local --list` after each run was
unchanged in all twelve runs.

| Extra line | Checks off: what Git did | Checks on |
|---|---|---|
| `option upload-pack touch-marker` | exit=0, ignored | `the box's branch listing is malformed` |
| `ok refs/heads/main` | exit=0, ignored | malformed |
| `@refs/heads/main HEAD` | exit=0, **created `refs/remotes/box/HEAD` → `refs/remotes/box/main`** | malformed; no `refs/remotes/box/HEAD` |
| `:object-format sha256` | exit=0, ignored | malformed |
| `aaaa…aaaa -x` | exit=0, ignored | malformed |
| empty line, then `ok refs/heads/main` | exit=1. Git read the box's lines as the helper's answer to `fetch`: "`warning: pinfold unexpectedly said: 'ok refs/heads/main'`", then the box's blank line as the end of the fetch, then "`error: pinfold::sim did not send all necessary objects`". No ref moved; the pack the helper unbundled afterwards stayed, a dangling commit | the rest became bundle bytes: `does not look like a v2 or v3 bundle file` |

Notes:
- Bundle headers: an unknown capability (F9c) and a wrong object format
  (F9b) were refused by `bundle verify`, and `@filter` by the helper
  (F9a).
- Push status lines (P5): with checks off the helper relayed the box's
  stdout ahead of its own status. Git printed "`warning: helper reported
  unexpected status of refs/heads/forged`" and exit=0. With checks on,
  the box's output was discarded.

### 5. Push

`capabilities` then `list for-push`, which the helper answered with an
empty list, then `push refs/heads/main:refs/heads/main` and a blank line
(`helper-P1-push-branch.log`). The helper bundled the source refs and
handed the bundle to `box_take_bundle`. The box fetched it with
`--atomic --no-tags --no-write-fetch-head` into `refs/remotes/host/*`.
The helper kept only the box's exit status (box stdout and stderr to
`/dev/null`).

- **P1:** exit=0. The box held `refs/remotes/host/main` at 9f4816c5 and
  its `refs/heads/main` was unchanged. **Git then moved the host's
  `refs/remotes/box/main` to 9f4816c5**, a value the box's branch never
  had. The next `git fetch box` reset it: "`+ 9f4816c...b703681 main       -> box/main  (forced update)`".
- **P2, `push box main:refs/tags/v1`:** "`! [remote rejected] main -> v1
  (only branches can be pushed)`", exit=1, no box ref.
- **P3, delete `:refs/heads/main`:** rejected the same way, exit=1.
- **P4, the box side failed** (not a repository):
  "`! [remote rejected] main -> main (the box refused the bundle)`",
  exit=1.
- **P6:**
  - `--atomic`: "`fatal: helper pinfold does not support --atomic`",
    exit=128.
  - `--dry-run`: "`fatal: helper pinfold does not support dry-run`",
    exit=128.
  - `--force`: exit=0.

### 6. Real boxes (`logs/e2e.log`, `logs/service.log`)

- **Setup.** The container service was stopped. It was started with
  `container system start --disable-kernel-install` and stopped with
  `container system stop`, twice: once for the discarded run below, not
  logged, and once for the logged run (`logs/service.log`). Each
  `container system stop` printed `buildkit` under "stopping
  containers", although `container list --all` showed it stopped before
  and after.
- **Helper version.** The e2e run used the helper before `PROBE_OWNREFS`
  was added and before the unchecked mode's unbundle moved to `fetch`.
  Both paths are off by default and unused by `e2e.sh`.
- **Image and state root.** `localhost/pinfold/profile-default:latest`
  was already in the store. `container image list` was the same before
  and after. The box spec named that image, a disk `from` a clone of
  the host repository, and no profile or egress. The XDG root was
  isolated, with a fresh `maintenance` stamp so the daily pass did not
  run.
- **E1, live box.** `box up` printed `ready` for `grh-probe-live`, and
  the box committed be2add3e. Then `git fetch box` with
  `PROBE_BOX=exec:grh-probe-live`: exit=0,
  "`* [new branch]      main       -> box/main`", and the host then held
  `refs/remotes/box/main` at be2add3e. The box committed a03af9f3 and
  was torn down: `"reason":"stdin-closed"`.
- **E2, stopped box.** `pinfold box exec grh-probe-live -- true` gave
  exit=3, "`no box named "grh-probe-live"`". Then `git fetch box` with
  `PROBE_BOX=transient:` started `grh-probe-transient` with the disk
  read-only, read the bundle and closed it. The helper log has
  `transient-ready ready grh-probe-transient` and `transient-down 0`.
  The fetch gave exit=0, "`be2add3..a03af9f  main       -> box/main`".
- **Cleanup.** `container list --all` afterwards showed only `buildkit`,
  stopped as before. `pinfold disk rm grh-probe-work` printed "`Removed
  grh-probe-work`".
- **Discarded run.** An earlier run of `e2e.sh`, made before the drivers
  unset the calling environment's `GIT_CONFIG_*` variables, had the
  same outcome. It was discarded and rerun; only the rerun is logged.

## Inferred

- **`import`** would have Git's `fast-import` write objects and refs into
  the repository directly from a stream the helper produces. The helper
  would have to turn the box's pack into a fast-import stream itself,
  with no quarantine. It has no advantage over `fetch` here.
- **`stateless-connect`** is the same shape as `connect` for this
  question: host Git speaks protocol v2 to a box-side service, and Git's
  own fetch code writes objects and refs.
- **No newline in a ref name.** Both the box's listing and a bundle
  header are split on newlines. So an "injected" line is just one more
  listed name, checked like the others.
- **Helper-written refs keep the transaction but not the report.** Git
  reads the old values of its refs before the helper's `fetch`, and its
  own update checks them. Any existing ref the helper moved then fails
  in Git's report, as in O2 and O3. Making it usable would need Git to
  skip refs the helper already wrote, which no helper capability for
  `fetch` offers.
- **The preflight is a check, not a transaction.** Git takes its own
  locks after the helper aborts its own. A ref or lock that appears in
  between still gives the R1 or R1b outcome. It narrows a host-side
  race; a box cannot trigger it at will, since the box's names are fixed
  at `list`.
- **Dangling objects after `--atomic`.** They are unreferenced, and
  `gc` prunes them after its expiry. "No object lands from a rejected
  bundle" holds for every bundle the helper rejects. It does not hold
  when Git's own ref update fails afterwards.
- **What must be documented as not enforced:** destinations chosen on
  the command line (`git fetch box SRC:DST`, `--refmap`), `git pull`,
  and plain fetch's partial update without `--atomic`. The helper
  enforces the box side (what is listed and what lands in the object
  store) and the configured refspec.
- **Git's own fetch behaviour applies after the helper.** Today's code
  turns off several fetch defaults, and Git applies them again:
  - `FETCH_HEAD` is written, unlike today's `--no-write-fetch-head` (F0
    shows its line).
  - `fetch.recurseSubmodules` and auto maintenance run as for any fetch.
    Neither was tested.
  - Git's connectivity check after `fetch` runs, since the helper
    advertises no `check-connectivity`. This is from the doc; not
    observed separately.
- **Output contract.** Callers get Git's output and exit codes: 128 for a
  helper refusal, 1 for a partial update, 253 and 254 for an atomic
  failure. They lose today's JSON lines and `refused`/`failed` reasons.
  That is a caller change.

## Not tried

- **Limits at full size:** 4 GiB bundles and 4096 branches.
- **Killing or stalling:**
  - killing the helper mid-fetch. Seen once by accident, not logged: a
    first draft of the unchecked mode hung, and the helper killed with
    `pkill` left its temp directory (a 503-byte `in.bundle`) under the
    system temp dir. It was removed by hand. No quarantine was involved
  - quarantine cleanup by dead pid; the prototype only removes its own
    quarantine on exit
  - a box that stalls or sends slowly
  - a real box killed past the limit; the prototype kills it in
    `local` mode only
- **Races:** concurrent fetches, and a race against the preflight.
- **Helper-written refs** when Git sends no `fetch` command (objects
  already present). Git would then write the refs itself.
- **Other Git paths:**
  - `--prune` and `fetch.prune`
  - `remote.box.followRemoteHEAD`, mirror remotes, `--refmap`
  - submodule recursion, auto maintenance
  - a sha256 host repository
  - `stateless-connect`
- **Platforms:** Linux and Firecracker.
- **Box behaviour:**
  - a tag in the box (the box script lists `refs/heads/` only)
  - relaying the box's stderr into an error detail
- **Pull with checks:** `git pull` with a hostile bundle. Fetch's checks
  apply, but the checkout is the documented exposure.

## Today's rules under a remote helper

Rules from `crates/pinfold/src/core/crossing.rs` and ARCHITECTURE.md
"Crossing".

| Rule | Remote helper | Evidence |
|---|---|---|
| Remote name valid like a profile name | Kept: argv[1] must be a plain name; a bare `pinfold::` URL is refused | R3f |
| Listing lines ≤ 4096 bytes, `OID refs/heads/NAME`, full OIDs | Kept | F7.2, N-*-checks-on |
| More than 4096 listed branches fails | Kept (scaled) | F6a |
| More than 4096 bundled heads fails | Kept (scaled) | F6b |
| Bundle past 4 GiB refused as `limit`, box killed | Kept for the limit (scaled); the kill not tested with a real box | F5 |
| Filtered bundle fails | Kept | F9a |
| `git bundle verify` | Kept | F9b, F9c, F9d |
| Every branch name passes `check-ref-format` under `refs/remotes/NAME/` | Kept for names; the destination Git writes is the refspec's, not checked | F7.*, F7b, R3a |
| `index-pack --fsck-objects` in a quarantine | Kept | F1, F2, F2b, F3 |
| Fetch's connectivity check | Kept, as the helper's `rev-list --objects` in the quarantine | F8b |
| Listed branch missing from the bundle lands only at a commit `refs/remotes/NAME/*` reaches | Kept | F8, F7.5 |
| Bundle holds only what `refs/remotes/NAME/*` lacks | Kept | F0b, E2 |
| A rejected bundle leaves the object store as it was | Kept for helper rejections; lost when Git's ref update fails after the helper (dangling pack) | F1–F9d; R1b, R2b |
| All refs in one `update-ref` transaction, prepared before packs move | Lost when Git writes the refs: partial without `--atomic`; a preflight narrows it. Kept if the helper writes them, but Git then reports every updated existing ref as failed (exit 1) | R1, R2, R1b, R2b, R1c, R2c, O1–O6 |
| Only `refs/remotes/NAME/*` is written | Kept for configured refspecs (refused otherwise); lost for command-line refspecs and `git pull` | R3a, R3c, R3d, R3e |
| Tags stay in the box | Kept by construction (only `refs/heads/` listed); a box tag not tested | — |
| No symref from the box | Kept by the helper's checks; Git creates `refs/remotes/box/HEAD` from a listed symref otherwise | N-symref |
| No hook or config from the box; nothing checked out | Kept for fetch; `git pull` checks out | N-* config, R3c |
| The repository's own hooks and config apply | Same | R3g |
| `--no-write-fetch-head` | Lost: `FETCH_HEAD` written | F0 |
| `--no-auto-maintenance`, `--no-recurse-submodules` | Lost to Git's defaults (inferred); not tested | — |
| A kill before the ref commit changes no ref; stale quarantines removed by pid | Not tested | — |
| JSON line and `refused`/`failed` reasons, exit 1 | Lost: Git's output and exit codes | all |
| Push: the host reads nothing from the box but success | Kept: empty `list for-push`, box output discarded | P1, P4, P5 |
| Push lands in the box's `refs/remotes/host/*`, `--atomic --no-tags` | Kept | P1 |
| Push sends every host branch | Changed: Git sends the refspecs the user names | P1 |
| Push only branches | Kept: tag destinations and deletes rejected | P2, P3 |
| Push leaves the host's refs alone | Lost: Git moves `refs/remotes/box/BRANCH` to the pushed value until the next fetch | P1 |
| `git push --atomic` | Refused by Git without the `option` capability | P6 |
| A stopped box is reachable | Kept through a transient box with the disk read-only | E2 |
