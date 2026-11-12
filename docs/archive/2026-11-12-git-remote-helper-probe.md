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
  file of hostile bytes (`feed:`), a shell script that never reads its
  stdin (`sh:`), a live box through `pinfold box exec` (`exec:`), or a
  transient box over a stopped box's disk (`transient:`).
- `git-remote-pinfoldconnect`: a ten-line `connect` helper, for comparison.
- `craft.py`: rewrites bundle headers for the hostile cases.
- `run.sh TEMP`: every simulated case, in order, from fresh repositories.
- `e2e.sh TEMP BINARY`: the two real-box fetches.
- `scrub.sh TEMP`: replaces the temp root (`TMP`), the system temp dir
  (`SYSTMP`), the home directory (`HOME`) and the account name (`user`)
  in the logs, run after the drivers.
- `logs/cases.log` (every simulated case), `logs/helper-CASE.log` (the
  helper's argv, `GIT_*` environment and every protocol line, control
  bytes escaped), `logs/e2e.log`, `logs/service.log`.

**Runs.** `logs/cases.log` and every `logs/helper-*.log` except
`helper-E*.log` come from one run of the current `run.sh` from a fresh
temp directory. The driver ran three times while the new cases were
written; only the last is logged. Between runs, one unlogged trial
outside `run.sh` tried refspec forms by hand (`refs/heads/*:`,
`^refs/heads/*`, none, `refs/heads/main`); the forms used are rerun in
G2b, G2d and G3 to G9. Outside the cases added in this pass,
`cases.log` matches the first logged run's except for object ids that embed
the temp path (F2, F2b, F3, C3), temp file names, the two added state
fields, the state diffs added to R3c and R3g, and a trailing space in
the N cases' notes. The helper logs were not compared; the O cases'
gained an `own-transaction-prepared` line. `logs/e2e.log`, `logs/service.log` and `logs/helper-E*.log`
predate this rerun. They used the helper as of commit d667c8b, and were
not rerun: no container ran in this pass.

Host Git ran with `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`,
and the drivers also unset the `GIT_CONFIG_PARAMETERS`,
`GIT_CONFIG_COUNT`, `KEY` and `VALUE` variables the calling environment
set. The helper logs no `GIT_CONFIG_VALUE_*`, and logs
`GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT` and `GIT_CONFIG_KEY_*` only
when a case sets `PROBE_LOG_CONFIG_ENV=1` (R3e). Size and branch limits
were scaled by environment variable (`PROBE_LIMIT=2000`,
`PROBE_BRANCHES=3`). The mechanism was tested, not 4 GiB or 4096
branches.

"State" below means a diff of `git count-objects -v`, `git for-each-ref`,
a sorted listing of every file under `.git/objects` plus any
`pinfold-incoming-*` quarantine directory, `git fsck` output with its
exit code, `.git/config`, and the name and hash of every file under
`.git/hooks`. The diff is taken before and after the command. Each case's
log lines are in `logs/cases.log` under `=== CASE`.

## Answer

**Capability: `fetch` (with `list`), plus `push`.** With `fetch`, the
helper reads and checks everything the box sends before Git sees a ref.
It kept these checks on the box's bytes:
- the listing and bundle limits, and the line and object id shape
- `bundle verify`
- the filtered-bundle refusal
- `index-pack --fsck-objects` in a quarantine
- a connectivity check inside the quarantine
- the reach rule for a listed branch the bundle leaves out
- ref-name checks

A bundle the helper rejects leaves no object and no ref. That held in
every hostile case the helper rejected.

**The single ref transaction is lost in three of the four variants
tried, and held in the fourth only for `git fetch` itself:**

- **Git writes the refs** (the normal shape). A box makes a plain
  `git fetch box` partial by renaming `x` to `x/y` and adding `z`, with
  no new object: Git wrote `refs/remotes/box/z`, failed `x/y`, exit 1
  (G1). With `--atomic` no ref moved (G1b). When objects are new, the
  `--atomic` failure leaves them in the repository (R1b).
- **The helper writes the refs at `fetch`** (O1 to O6). Git does not
  send `fetch` when every listed object is already present, so the
  helper never runs its transaction: G2 was partial exactly like G1.
  When it does run, Git then fails its own update of every existing ref
  it moved, exit 1 (O2, O3).
- **The helper writes the refs at `list`, and Git writes none.** This
  needs the configured refspec `^refs/heads/*` alone; a glob with no
  destination is an invalid refspec (G2b), and with no refspec Git fails
  after the helper has written (G2d). With `^refs/heads/*` the
  transaction held: a D/F rename was refused with exit 128 and no object
  or ref changed (G3, G3b), and an honest fetch exited 0 (G4). The
  costs: `git fetch` then prints nothing, `git ls-remote box` and
  `git fetch --dry-run box` also write `refs/remotes/box/*` (G6, G6b),
  `git fetch box main:other` still writes `refs/heads/other` (G5), and
  a Git failure after `list` leaves the helper's refs written (G2d).
- **The helper writes the refs at Git's closing blank line** (G7 to G9).
  Git exited 0 although the helper's transaction failed (G8, G8b), and
  `ls-remote` writes refs too (G9).

The helper sees configured refspecs and can refuse them. It cannot see a
command-line refspec: `git fetch box main:other` wrote
`refs/heads/other`, and `git pull box main` moved the host's `main` and
its working tree.

`connect` is ruled out: host Git took a malformed commit and a `.git`
tree entry straight into the object store.

## Observed

### 1. Capabilities: who writes objects and refs

- **fetch.** `capabilities` then `list` then a batch of `fetch OID NAME`
  lines (`helper-F0-baseline.log`). The helper made the objects present:
  its quarantined packs were moved in at `fetch`. Git then wrote
  `refs/remotes/box/main` ("`* [new branch]      main       -> box/main`").
  Git then checks that the fetched refs' objects are present. It failed
  with "`error: pinfold::sim did not send all necessary objects`" when
  they were not: in N-early-blank and N-short-oid with checks off, and
  in M1 to M3 with checks off.
- **No `fetch` command.** When every listed object is already in the
  repository, Git sends `capabilities`, `list`, then a blank line, and
  writes the refs itself (`helper-G1-no-fetch-plain.log`).
- **connect** (C1, `-c fetch.fsckObjects` unset). Host Git spoke the pack
  protocol to `git upload-pack` on the box repository. It wrote the
  objects and refs itself:
  - The malformed commit landed: "`bad commit
    58a90eb1c08faeefd7bc75eb6baf571937f3a848 on the host: commit`".
  - The host's fsck then failed: "`error in commit
    58a90eb1c08faeefd7bc75eb6baf571937f3a848: missingEmail: invalid
    author/committer line - missing email`".
  - Git also created `refs/remotes/conn/HEAD`.

  C3: a tree with a `.git` entry landed, and fsck printed a `warning in
  tree` line ending "`hasDotgit: contains '.git'`". C2, with
  `fetch.fsckObjects=true`: refused, "`fatal: fsck error in packed
  object`", exit=128. That run still left a `./pack/tmp_pack_*` file in
  the object store (`garbage: 1`).
- **import.** Not tried; see Inferred.

### 2. Hostile bundles with the `fetch` helper

Each case: `git -C host fetch box`. Each ended with exit=128 and "state:
unchanged (count-objects, for-each-ref, object files, fsck, .git/config,
.git/hooks)". The helper's message:

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
| F9b | header `@object-format=sha256` over a sha1 head line | `git bundle failed (exit 1): error: unrecognized header: b7036811f28d630ef484116a6df59f77f15c520f refs/heads/main (56)` |
| F9c | header `@hooks=post-checkout` | `git bundle failed (exit 1): error: unknown capability 'hooks=post-checkout'` |
| F9d | prerequisite the host lacks | `git bundle failed (exit 1): ` (`bundle verify -q` printed nothing) |
| K1 | listed `stolen` at a commit on a host-only branch, no bundle | `branch 'stolen' names 80738132c8db4dc3318f77fa45cec96a926152c6, which the bundle does not carry and refs/remotes/box/* does not reach` |
| M1 | pack lacks the head's parent commit | `git rev-list failed (exit 128): error: Could not read b7036811f28d630ef484116a6df59f77f15c520f` |
| M2 | pack lacks the head's tree | `git rev-list failed (exit 128): fatal: bad tree object 796307bff0c2cd371ded6d50688ba4a6a1ad31e1` |
| M3 | pack lacks a blob | `git rev-list failed (exit 128): fatal: missing blob object 'd591a2710eb6ec0dc090ed021171bc58abca0f95'` |
| L1 | bundle of 300000 random bytes, past the read cap | `the bundle is past 2000 bytes` |

More cases:
- F7.5: a listed `refs/heads/-x` that the bundle did not carry. It was
  refused by the reach rule, not by its name.
- F7c: a real box branch `-x` passed `check-ref-format`. It landed as
  `refs/remotes/box/-x`, exit=0.
- F9e: a prerequisite the host holds, with a 100000-byte subject, was
  accepted. The header reader keeps 8 bytes of each line.
- K1b, control for K1: a listed `alias` at the box's `main`, which
  `refs/remotes/box/main` reaches, landed: "`* [new branch]      alias
  -> box/alias`", exit=0.
- M1 to M3: the helper's `rev-list --objects` in the quarantine refused
  each. It runs after `bundle verify` and `index-pack --fsck-objects`,
  so those passed; that is read from the helper's step order and the
  `git rev-list failed` messages, not logged per step. With checks off (`git bundle unbundle` at `fetch`), Git refused
  each with exit=1 and "`did not send all necessary objects`", and the
  objects stayed in the repository (fsck: "`dangling commit
  52960449ecdbf8fb51d6fee8b0d0a3ae37f97084`" in M1).
- L1: the box's stream passed the helper's read cap
  (`2000+1+4096*4 = 18385` bytes), so the helper killed the box:
  `box-fetch-exit -9`. The bundle writer survived the kill
  (`bundle-writers-after-kill 1`), and was gone once the helper had
  exited ("`bundle writers still running after the helper exited: 0`").
  In F5 the stream fitted the cap and the box exited 0
  (`helper-F5-size-limit.log`).
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

**(b) No `fetch` command.** In G1, the host held `refs/remotes/box/x`.
The box renamed `x` to `x/y` and added `z` at `main`, so no object was
new. Git sent no `fetch` (`helper-G1-no-fetch-plain.log`) and wrote the
refs itself.

- **G1, plain:** exit=1, "`! [new branch]      x/y        -> box/x/y  (unable to update local ref)`"
  and "`* [new branch]      z          -> box/z`". `refs/remotes/box/z` was
  created. Partial.
- **G1b, `--atomic`:** exit=254, "`error: cannot lock ref
  'refs/remotes/box/x/y': 'refs/remotes/box/x' exists; cannot create
  'refs/remotes/box/x/y'`". State unchanged. Git first printed both
  `[new branch]` lines as if they had landed.
- **G2, the same with `PROBE_OWNREFS=fetch`:** the same output as G1,
  exit=1, `refs/remotes/box/z` created. The helper's transaction never
  ran.

**(c) The helper writes `refs/remotes/box/*` itself.** It runs today's
`update_refs` sequence: `start`, the updates and `prepare` against the
quarantine, then the pack move, then `commit`. `PROBE_OWNREFS` says
when.

- **At `fetch`, Git's default refspec** (O1 to O6):
  - O1, a new ref: exit=0, `refs/remotes/box/main` at b7036811.
  - O2, a fast-forward: exit=1, "`error: cannot lock ref
    'refs/remotes/box/main': is at 52960449ecdbf8fb51d6fee8b0d0a3ae37f97084
    but expected b7036811f28d630ef484116a6df59f77f15c520f`". The ref was
    left at the new value 52960449, the right one.
  - O3, a rewritten box branch: the same, "`is at
    d876e12af719d57ae3e80f47829d2c98ba293e90 but expected
    b7036811f28d630ef484116a6df59f77f15c520f`", exit=1. The ref was at
    d876e12a.
  - O4 (plain) and O5 (`--atomic`), the D/F case with a new object:
    "`own ref transaction failed: fatal: prepare: cannot lock ref
    'refs/remotes/box/x/y'`", exit=128, state unchanged in both.
  - O6, `git fetch box main:other`: exit=0. Git still wrote
    `refs/heads/other`.
- **Refspecs that make Git write nothing** (all with
  `PROBE_OWNREFS=list`):
  - G2b, `remote.box.fetch=refs/heads/*`: "`fatal: invalid refspec
    'refs/heads/*'`", exit=128. The helper was not started.
  - G2c, no `remote.box.fetch`, the G1 rename: the helper's transaction
    failed at `list`, exit=128, state unchanged.
  - G2d, no `remote.box.fetch`, two new branches: the helper wrote
    `refs/remotes/box/main` and `refs/remotes/box/other` and moved the
    pack. Then Git failed:
    "`fatal: couldn't find remote ref HEAD`", exit=128.
  - The cases below use `remote.box.fetch=^refs/heads/*` alone. Git then
    asked for no ref and sent no `fetch`.
- **At `list`** (`PROBE_OWNREFS=list`):
  - G3, the G1 rename: "`own ref transaction failed: fatal: prepare:
    cannot lock ref 'refs/remotes/box/x/y': 'refs/remotes/box/x' exists;
    cannot create 'refs/remotes/box/x/y'`", exit=128, state unchanged.
  - G3b, the R1 change with a new object: the same message, exit=128,
    state unchanged; the pack did not land.
  - G4, a fast-forward of `main` plus a new `other`: exit=0, both refs
    at 52960449. Git printed nothing.
  - G4b, `x` renamed to `w`: exit=0, `refs/remotes/box/w` created,
    `refs/remotes/box/x` kept.
  - G5, `git fetch box main:other`: "`* [new branch]      main       ->
    other`", exit=0. `refs/heads/other` and `refs/remotes/box/main` were
    both written.
  - G6, `git ls-remote box`: exit=0, and `refs/remotes/box/main` was
    created.
  - G6b, `git fetch --dry-run box`: exit=0, and `refs/remotes/box/main`
    was created.
- **At Git's closing blank line** (`PROBE_OWNREFS=end`):
  - G7, a fast-forward: exit=0, `refs/remotes/box/main` at 52960449. Git
    printed nothing.
  - G8 (with a new object) and G8b (none): the helper printed "`own ref
    transaction failed: fatal: prepare: cannot lock ref
    'refs/remotes/box/x/y'`", and `git fetch box` gave exit=0 in both.
    State unchanged.
  - G9, `git ls-remote box`: exit=0, and `refs/remotes/box/main` was
    created.

**(d) Atomicity.** `git help -c` lists no `fetch.atomic` (D1 logs its
`fetch.*` names). Atomicity is the caller's `--atomic` flag. From the
gitremote-helpers documentation, not tested: a helper that advertises no
`option` capability, as this one, is never told whether the flag was
given.

**(e) Refspecs that write `refs/heads/*`.**

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
- **R3e, `git -c 'remote.box.fetch=+refs/heads/*:refs/heads/box-*'`:**
  seen too, through `GIT_CONFIG_PARAMETERS`, and refused: "`remote.box.fetch
  writes outside refs/remotes/box/: +refs/heads/*:refs/heads/box-*`",
  exit=128.
- **R3f, `git fetch pinfold::sim main:other`:** the helper got
  `argv ['pinfold::sim', 'sim']`. It refused, since the first argument is
  not a plain remote name. "`fatal: remote helper 'pinfold' aborted
  session`", exit=128. So a fetch with no configured remote never runs.
- **R3h, the URL part:** remotes with URLs `pinfold::../x` and
  `pinfold::-x` were refused, "`box name '../x' in the URL is not a plain
  name`" and "`box name '-x' in the URL is not a plain name`", exit=128
  each. The first logged run had no such check.

**(f) Prune, clone, a branch named `HEAD`.**

- **Q1, `git fetch --prune box`** after the box deleted `gone`:
  "`- [deleted]         (none)     -> box/gone`", exit=0, and
  `refs/remotes/box/gone` was removed. Q2, `-c fetch.prune=true`: the
  same. Q1b, control without prune: the ref stayed, exit=0.
- **Q3, `git clone pinfold::sim TMP/p/clone`:** the helper got
  `argv ['origin', 'sim']` and its `git config --get-all
  remote.origin.fetch` returned nothing (`config-fetch` is empty in
  `helper-Q3-clone.log`), so the refspec gate saw no refspec. Git wrote
  `refs/remotes/origin/main` and `refs/remotes/origin/other`, printed
  "`warning: remote HEAD refers to nonexistent ref, unable to
  checkout`", and exited 0. The clone held only `.git`; HEAD was
  `refs/heads/master` with no commits.
- **Q4, a box branch `refs/heads/HEAD`** at a commit other than `main`
  (cc7d0d40): it passed every check and landed as the plain ref
  `refs/remotes/box/HEAD`. `git rev-parse box` then gave
  `cc7d0d40a20614617c3e25653eef9665c8b18351`, not the box's `main`.

### 4. What reaches host Git from the box

For each `N-` case the listing carried extra lines. Each was run with the
helper's checks off (`PROBE_CHECKS=off`, the box's listing passed to Git
verbatim) and on. State, `.git/config` included, was diffed in every
run.

| Extra lines | Checks off: what Git did | Checks on |
|---|---|---|
| `option upload-pack touch-marker` | exit=0. Git asked to fetch only `refs/heads/main`; no ref and no config came from the line | `the box's branch listing is malformed` |
| `ok refs/heads/main` | the same | malformed |
| `@refs/heads/main HEAD` | exit=0, **created `refs/remotes/box/HEAD` → `refs/remotes/box/main`** | malformed; no `refs/remotes/box/HEAD` |
| `:object-format sha256` | exit=0, only `refs/heads/main` fetched | malformed |
| `aaaa…aaaa -x` | exit=0, only `refs/heads/main` fetched | malformed |
| empty line, then `ok refs/heads/main` (two lines) | exit=1. Git read the box's lines as the helper's answer to `fetch`: "`warning: pinfold unexpectedly said: 'ok refs/heads/main'`", then the box's blank line as the end of the fetch, then "`error: pinfold::sim did not send all necessary objects`". No ref moved; the pack the helper unbundled afterwards stayed, a dangling commit | the rest became bundle bytes: `does not look like a v2 or v3 bundle file` |
| `OID refs/heads/xxx…`, a name of `refs/heads/` plus 5000 `x` (`run.sh`) | exit=255. Git printed `[new branch]` for `main` and the long name, failed to lock the long ref, and moved no ref; the pack stayed, a dangling commit | malformed |
| `b703681 refs/heads/short` | exit=1, "`fatal: bad object b703680000000000000000000000000000000000`", then "`did not send all necessary objects`" | malformed |

Why Git ignored the first, second, fourth and fifth lines is not
logged. The helper log shows only that Git's `fetch` batch named
`refs/heads/main` alone.

Notes:
- Bundle headers: an unknown capability (F9c) and a sha1 head line under
  `@object-format=sha256` (F9b) were refused by `bundle verify`, and
  `@filter` by the helper (F9a).
- Push status lines (P5): with checks off the helper relayed the box's
  stdout ahead of its own status. Git printed "`warning: helper reported
  unexpected status of refs/heads/forged`" and exit=0. With checks on,
  the box's output was discarded.

### 5. Control bytes from the box

Host stderr was captured to a file, logged through `cat -v`, and its raw
ESC and CR bytes counted.

| Case | Box-controlled string | What reached stderr | Raw ESC, CR |
|---|---|---|---|
| X1 | bundle capability `@x` ESC `[31mRED` CR `cap` | `error: unknown capability 'x?[31mRED?cap'` | 0, 0 |
| X2 | bundle header line ESC `[31mnot-a-header` CR | `error: unrecognized header: ?[31mnot-a-header (17)` | 0, 0 |
| X3, checks on | listed `refs/heads/a` ESC `[31mb` CR | `the box named a malformed branch: 'a\x1b[31mb\r'` | 0, 0 |
| X3, checks off | the same | `error: * Ignoring funny ref 'refs/remotes/box/a?[31mb' locally`; `main` landed, exit=0 | 0, 0 |
| X4 | `.gitmodules` url `--x` ESC `[31mRED` CR | `gitmodulesUrl: disallowed submodule url: --x?[31mRED` | 0, 0 |

Git printed each ESC as `?` in its own messages. No CR reached stderr
raw: X1's CR became `?`, while X2's and X4's were dropped (no
`?` after `not-a-header` or `RED`). The helper relays Git's stderr as it
is, and quotes ref names with Python's `repr`.

### 6. Hooks and config

`.git/config` and `.git/hooks` were in every state diff above. No case
changed either. The cases that need a refspec (R3d, G2b to G9) set it
before the first snapshot.

- **H1.** The host installed `reference-transaction` and `post-merge`
  hooks that append `HOST-…` lines to a marker. The box committed an
  executable `hooks/post-merge` that would append `BOX-post-merge`.
  `git fetch box` ran the host's `reference-transaction` hook
  (`preparing`, `prepared`, `committed`). `git pull --no-rebase box main`
  fast-forwarded and checked out `hooks/post-merge` in the working tree,
  and the marker gained `HOST-post-merge-0`. No `BOX-` line appeared,
  and `.git/hooks` was unchanged by both.
- **H2.** With checks off, a `.git/hooks/post-checkout` tree entry
  landed in the object store (fsck "`hasDotgit: contains '.git'`"
  warning), `.git/hooks` was unchanged, and the marker was absent.
- **R3g, control:** the host's own `reference-transaction` hook ran on
  `git fetch box`.

### 7. Push

`capabilities` then `list for-push`, which the helper answered with an
empty list, then `push refs/heads/main:refs/heads/main` and a blank line
(`helper-P1-push-branch.log`). The helper bundled the source refs and
handed the bundle to `box_take_bundle`. The helper kept only the box's
exit status (box stdout and stderr to `/dev/null`).

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

### 8. Stalls

The driver gave the host 2000 refs under `refs/remotes/box/`
(`run.sh`), so the helper's held list is 2000 lines of 41 bytes, 82000
bytes by arithmetic, larger than a pipe buffer. Neither number is
logged.

- **S1, a box that exits without reading** (`local:` at a path that is
  not a repository): the helper died with a Python traceback ending
  "`BrokenPipeError: [Errno 32] Broken pipe`", exit=128. State
  unchanged.
- **S2, a box that never reads** (`sh:exec sleep 8`), with Git killed by
  `SIGALRM` after 4 s: "`git returned after 4 s`", exit=142. The helper
  stayed blocked writing to the box until the box exited ("`helper gone
  8 s after the start`"), then died with the same traceback.

### 9. Real boxes (`logs/e2e.log`, `logs/service.log`)

These predate this pass and used the helper as of d667c8b.

- **Setup.** The container service was stopped before the session. It
  was started with `container system start --disable-kernel-install`
  and stopped with `container system stop` for each of two runs.
  `logs/service.log` holds the logged run's start and stop. That stop
  printed `buildkit` under "stopping containers", although `container
  list --all` showed it stopped before and after. The discarded run's
  start and stop are not logged.
- **Helper version.** The e2e run used the helper before `PROBE_OWNREFS`
  was added and before the unchecked mode's unbundle moved to `fetch`.
  Both paths are off by default and unused by `e2e.sh`. It also predates
  the URL-part check and the logging switch.
- **Image and state root.** `localhost/pinfold/profile-default:latest`
  was already in the store. `container image list` was the same before
  and after. The box spec named that image, a disk `from` a clone of
  the host repository, and no profile or egress. The XDG root was
  isolated, with a fresh `maintenance` stamp (`e2e.sh`). That the daily
  pass then did not run is not logged.
- **E1, live box.** `box up` printed `ready` for `grh-probe-live`, and
  the box committed be2add3e. Then `git fetch box` with
  `PROBE_BOX=exec:grh-probe-live`: exit=0,
  "`* [new branch]      main       -> box/main`", and the host then held
  `refs/remotes/box/main` at be2add3e. The box committed a03af9f3 and
  was torn down: `"reason":"stdin-closed"`.
- **E2, stopped box.** `pinfold box exec grh-probe-live -- true` gave
  exit=3, "`no box named "grh-probe-live"`". Then `git fetch box` with
  `PROBE_BOX=transient:` started `grh-probe-transient`, read the bundle
  and closed it. Its spec asked for the disk read-only
  (`"readonly":true` in `e2e.sh`); the spec is not logged and read-only
  access was not checked. The helper log has `transient-ready ready
  grh-probe-transient` and `transient-down 0`. The fetch gave exit=0,
  "`be2add3..a03af9f  main       -> box/main`".
- **Cleanup.** `container list --all` afterwards showed only `buildkit`,
  stopped as before. `pinfold disk rm grh-probe-work` printed "`Removed
  grh-probe-work`".
- **Discarded run.** An earlier run of `e2e.sh`, made before the drivers
  unset the calling environment's `GIT_CONFIG_*` variables, was
  discarded and rerun. Its output is not logged; only the rerun is.

## Inferred

- **`connect` loses pinfold's checks.** Host Git reads the pack itself,
  so nothing the helper could check sits between the box and the object
  store: the size and branch limits, the listing shape, branches only,
  the filtered-bundle refusal and the quarantine. fsck becomes the
  repository's config, not pinfold's. Only fsck's absence (C1, C3) was
  observed.
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
- **F7c and Q4 match today's rule.** Today's `check_branch` runs the same
  `check-ref-format` under `refs/remotes/NAME/`, so it would accept `-x`
  and `HEAD` too. Read from the code, not run.
- **The connectivity check differs from today's.** Today's fetch runs a
  quarantined `git fetch` of the bundle; the prototype runs
  `rev-list --objects` in the quarantine. M1 to M3 show the prototype's
  refuses a missing commit, tree and blob. That the two are equivalent
  is not shown.
- **Helper-written refs at `fetch` keep the transaction but not the
  report.** Git reads the old values of its refs before the helper's
  `fetch`, and its own update checks them. Any existing ref the helper
  moved then fails in Git's report, as in O2 and O3.
- **Helper-written refs at `list` turn every `list` into a write.**
  `ls-remote` and `--dry-run` wrote refs (G6, G6b); so would any other
  command that lists the remote. Git's report says nothing about refs it
  did not write.
- **The preflight is a check, not a transaction.** Git takes its own
  locks after the helper aborts its own. A ref or lock that appears in
  between still gives the R1 or R1b outcome. It narrows a host-side
  race; a box cannot trigger it at will, since the box's names are fixed
  at `list`. It does not help when Git sends no `fetch` (G1).
- **Dangling objects after `--atomic`.** They are unreferenced, and
  `gc` prunes them after its expiry. "No object lands from a rejected
  bundle" holds for every bundle the helper rejects. It does not hold
  when Git's own ref update fails afterwards.
- **The author's reading of what would need documenting as not
  enforced:** destinations chosen on the command line (`git fetch box
  SRC:DST`, `--refmap`), `git pull`, `git clone`'s refspec, `--prune`,
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
    advertises no `check-connectivity`; it was observed failing when
    objects were missing (N-early-blank, N-short-oid, M1 to M3, checks
    off).
- **Control bytes.** Git's own messages replaced them with `?` in every
  case tried (X1 to X4), and the prototype's own messages use `repr`.
  A helper that printed a box-controlled string itself without quoting
  would pass raw bytes through; none of the prototype's paths does.
  Today's code prints a crossing error's `detail` through
  `serde_json::json!` (`crates/pinfold/src/cli.rs`, `refused` and the
  fetch's `failed` arm), and serde_json writes control characters as
  `\u` escapes. Read from the code, not run.
- **Output contract.** Callers get Git's output and exit codes: 128 for a
  helper refusal, 1 for a partial update, 253 and 254 for an atomic
  failure, and 0 for a failed transaction at the closing line (G8).
  They lose today's JSON lines and `refused`/`failed` reasons. That is a
  caller change.

## Not tried

- **Limits at full size:** 4 GiB bundles and 4096 branches.
- **Killing or stalling:**
  - killing the helper mid-fetch. Seen once by accident, not logged: a
    first draft of the unchecked mode hung, and the helper killed with
    `pkill` left its temp directory (a 503-byte `in.bundle`) under the
    system temp dir. It was removed by hand. No quarantine was involved
  - quarantine cleanup by dead pid; the prototype only removes its own
    quarantine on exit
  - a box that sends slowly
  - the read-cap kill with a real box. The kill path is shared by
    `local:`, `exec:` and `transient:`; only `local:` was exercised (L1)
- **Races:** concurrent fetches, and a race against the preflight.
- **Other Git paths:**
  - `remote.box.followRemoteHEAD`, mirror remotes, `--refmap`
  - `git clone -c remote.origin.fetch=…`
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
| Remote name valid like a profile name | Kept: argv[1] must be a plain name; a bare `pinfold::` URL is refused. The URL part (the box name) is checked only since this pass | R3f, R3h |
| Listing lines ≤ 4096 bytes, `OID refs/heads/NAME`, full OIDs | Kept | F7.2, N-*-checks-on, N-long-line, N-short-oid |
| More than 4096 listed branches fails | Kept (scaled) | F6a |
| More than 4096 bundled heads fails | Kept (scaled) | F6b |
| Bundle past 4 GiB refused as `limit`, box killed | Kept (scaled); the kill fired in `local:` mode only, not with a real box | F5, L1 |
| Filtered bundle fails | Kept | F9a |
| `git bundle verify` | Kept | F9b, F9c, F9d |
| Every branch name passes `check-ref-format` under `refs/remotes/NAME/` | Kept for names; the destination Git writes is the refspec's, not checked | F7.*, F7b, R3a |
| `index-pack --fsck-objects` in a quarantine | Kept | F1, F2, F2b, F3 |
| Fetch's connectivity check, in the quarantine | Kept, as the helper's `rev-list --objects` in the quarantine; its equivalence to today's quarantined `git fetch` is inferred | F8b, M1, M2, M3 |
| Listed branch missing from the bundle lands only at a commit `refs/remotes/NAME/*` reaches | Kept | F8, F7.5, K1, K1b |
| Bundle holds only what `refs/remotes/NAME/*` lacks | Kept | F0b, E2 |
| A rejected bundle leaves the object store as it was | Kept for helper rejections; lost when Git's ref update fails after the helper (dangling pack) | F1–F9d except F7c and F9e, K1, M1–M3; R1b, R2b |
| All refs in one `update-ref` transaction, prepared before packs move | Lost in three variants, held in one at a cost. When Git writes the refs, a box makes a plain fetch partial, with or without new objects. Helper-written refs: at `fetch` the transaction is skipped when no object is new, and Git reports moved refs as failed; at `list` it holds for `git fetch`, but `ls-remote` and `--dry-run` write refs, `git fetch` prints nothing, and a Git failure after `list` leaves the refs written; at the closing line Git exits 0 on failure | R1, R2, G1, G1b, G2, O2, O3, G2d, G3, G3b, G4, G6, G6b, G8, G8b |
| Only `refs/remotes/NAME/*` is written | Kept for configured refspecs (refused otherwise); lost for command-line refspecs and `git pull`. During `git clone` the gate saw no refspec, so clone's refspec is unchecked; Q3 wrote only `refs/remotes/origin/*` | R3a, R3c, R3d, R3e, G5, Q3 |
| A tracking ref is never deleted | Changed: `--prune` and `fetch.prune` delete `refs/remotes/box/*` the box no longer lists | Q1, Q2, Q1b |
| Tags stay in the box | Kept by construction (only `refs/heads/` listed); a box tag not tested | — |
| No symref from the box | Kept by the helper's checks; Git creates `refs/remotes/box/HEAD` from a listed symref otherwise. A box branch named `HEAD` lands as a plain `refs/remotes/box/HEAD`, which `box` then names (inferred the same today) | N-symref, Q4 |
| No hook or config from the box; nothing checked out | Kept for fetch: no case changed `.git/config` or `.git/hooks`; `git pull` checks out | N-*, H1, H2, R3c |
| The repository's own hooks and config apply | Same | R3g, H1 |
| Box-controlled text reaches the caller escaped (JSON `detail`, from the code) | Kept in the cases tried: Git prints `?` for control bytes and the helper quotes names with `repr`; not a property of the helper's design | X1–X4 |
| `--no-write-fetch-head` | Lost: `FETCH_HEAD` written | F0 |
| `--no-auto-maintenance`, `--no-recurse-submodules` | Lost to Git's defaults (inferred); not tested | — |
| A box that stops reading only fails its own fetch (stdin written from a thread) | Missing from the prototype: it writes the held list inline, so a box that never reads blocks it until the box exits; one that exits gives a traceback | S1, S2 |
| A kill before the ref commit changes no ref; stale quarantines removed by pid | Not tested; the prototype has no pid sweep | — |
| `migrate` undoes a partial pack move, and fails when another fetch swept the quarantine | Missing from the prototype: it moves packs without undo, and returns as if empty when the quarantine is gone | — |
| JSON line and `refused`/`failed` reasons, exit 1 | Lost: Git's output and exit codes | all |
| Push: the host reads nothing from the box but success | Kept: empty `list for-push`, box output discarded | P1, P4, P5 |
| Push lands in the box's `refs/remotes/host/*`, `--atomic --no-tags` | Kept for the destination (P1). The flags are in the prototype's `PUSH_SCRIPT`, not observed | P1 |
| Push sends every host branch | Changed: Git sends the refspecs the user names | P1 |
| Push only branches | Kept: tag destinations and deletes rejected | P2, P3 |
| Push leaves the host's refs alone | Lost: Git moves `refs/remotes/box/BRANCH` to the pushed value until the next fetch | P1 |
| `git push --atomic` | Refused by Git without the `option` capability | P6 |
| A stopped box is reachable | Kept through a transient box whose spec asks for the disk read-only; read-only access not checked | E2 |
