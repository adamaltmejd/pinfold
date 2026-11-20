# Host tags into the box; the helper link (2026-11-20)

Two changes on one branch: host tags may be pushed into the box (#364),
and pinfold creates the `git-remote-pinfold` link itself. The test suite
is suspended, so each was checked through the real binary on macOS
(Apple container 1.7.1, host git 2.56.1) from a debug build at the
branch tip.

## Why tags were excluded

Push has carried branches only since the first crossing (#135,
2026-10-20). That spec and the helper probe
(`2026-11-12-git-remote-helper-probe.md`, row "Push only branches")
kept the branch-only scope from the earlier design. They give no reason
tied to the push direction. Each stated reason for keeping tags out is
about fetch:

- a box tag must not reach the host;
- a box tag must not shadow a host branch;
- `fetch.pruneTags` once let the inner fetch write and delete host tags
  (`2026-11-19-git-remote.md`, review fix 1).

Push moves host data into the box, and host data is trusted in that
direction. Fetch is unchanged: the box lists and bundles `refs/heads/*`
only.

## Design

- `push refs/tags/SRC:refs/tags/TAG` becomes the refspec
  `[+]refs/tags/SRC:refs/tags/TAG` in the box. Branches stay
  `+refs/heads/SRC:refs/remotes/host/BRANCH`. Any other pairing is
  rejected. So are deletions and a stash or other source.
- A non-forced tag that clobbers an existing one fails Git's atomic
  fetch whole. Plain git 2.56.1 showed this: `! [rejected] v1 -> v1
  (would clobber existing tag)`, exit 1, and `host/main` was rolled back
  too.
- The box script checks before it changes anything. For each unforced
  refspec, it compares the destination that `show-ref --verify` finds
  with that ref's head in the bundle. If any differs, the script exits
  65 and changes nothing.
- The host maps only exit 65 to `a pushed tag already exists in the box
  with a different target; nothing was pushed (use --force to replace
  box tags)`. Every ref of the push is rejected with that reason. Any
  other nonzero code is still "the box refused the push", and only 0 is
  success.
- If a tag appears between the check and the fetch, the atomic fetch
  rejects the clobber and fails every ref. The plain-git probe above
  shows that. The host reads nothing from the box but its exit status,
  and box stdout is discarded as before.
- A first version had the box print the conflicting destinations so
  the rest of the push could land. The operator vetoed it, because it
  added a reply channel from the box.
- `list for-push` first answered with an empty list, so every
  followTags push resent every reachable tag. The operator then asked
  for quiet repeats, reusing the fetch listing.
- Final design: `list for-push` runs one more box command,
  `PUSHED_SCRIPT`. It prints `OID REF` lines for the box's
  `refs/remotes/host/` and `refs/tags/`, then an empty line, which is
  the fetch listing's format.
- That output goes through the same `receive` and `Receiver`. The
  checks are the same:
  - lines of at most 4096 bytes;
  - full-width object ids;
  - at most 4096 refs;
  - UTF-8.
  The only new parameter is the allowed prefixes. Fetch passes
  `refs/heads/` only, and its behaviour is unchanged.
- Any bundle bytes after the empty line fail the listing.
- The host maps `refs/remotes/host/NAME` to `refs/heads/NAME`. Every
  name must pass `check-ref-format` and appear once.
- `pushed_cancellable` returns a plain map, with no quarantine and no
  objects. The helper never stores it in `fetched`, so it cannot feed a
  fetch.
- Failure handling: a box without a repository (exit 64), a listing
  that fails a check, or any other crossing error gives an empty list,
  and the push goes ahead as before. I chose this over refusing because
  the push itself still runs every check and reports a real failure.
  After review, this also covers errors outside the box command:
  - the box ending during the listing;
  - a refused remote;
  - no project disk.
  Each lists nothing, and the push's own crossing then reports the
  failure if it persists. Only a signal (`Interrupted`) still stops the
  helper at `list for-push`.
- Final filter (round 3): after validation, pinfold keeps an entry only
  if its id is the id of a host ref under `refs/heads/*` or `refs/tags/*`
  (a tag ref's own id). It reads those ids from one `git for-each-ref`.
  - pinfold looks up no box-supplied id.
  - Every id Git then sees is one a host ref points at, so host Git has
    nothing to fetch, in a partial clone or otherwise, on any Git
    version.
  - This replaced round 2's `cat-file --batch-check` with
    `GIT_NO_LAZY_FETCH=1` and its promisor detection. Review found that
    detection leaky on Git 2.43: `remote.*.partialclonefilter` alone
    registers a promisor, and `extensions.partialClone = false` names a
    remote.
  - The filter went from 44 lines (the call plus a 43-line function) to
    11 inline.
- Git plans the push from what remains, with these effects:
  - an unchanged branch or tag prints `Everything up-to-date`;
  - a box `host/NAME` at an older host commit that is no longer a ref
    tip looks absent, so Git prints `* [new branch]` instead of a range
    and pushes;
  - a box tag at an object no host ref points at looks absent, so
    followTags sends the host's tag of that name, and the box's
    exit-65 check fails the whole push unless forced;
  - a box tag of that name at another host tip is listed, so followTags
    skips it, and Git rejects an explicit push of it (`already exists`)
    unless forced;
  - host Git may reject a branch push it can see is not a fast-forward
    of the box's `host/NAME` (`non-fast-forward`). pinfold forces every
    branch refspec either way.
- The box script's exit-65 check also covers a tag created after the
  listing, and an empty listing.

## Commands

The setup was:

- `R` was a temp dir under the worktree's ignored `target/`, used
  because the sandbox refuses git outside the worktree. It held:
  - `R/bin/pinfold`, a copy of the debug binary;
  - `R/xdg/{config,state,cache}`;
  - `R/repo`.
- A wrapper `R/pf` set the XDG roots, `PINFOLD_PROFILE=default`,
  `VISUAL=true EDITOR=true`, `PINFOLD_NO_UPDATE_CHECK=1` and
  `PATH=R/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin`. The
  only `git-remote-pinfold` on that PATH was the one in `R/bin`.
- `expect` answered the prompts.

The repo had branches `main`, `side` and `topic`. Its tags were:

- `v1` and `v2`, annotated, on main;
- `light`, lightweight;
- `v3-topic`, annotated, on topic only.

### Setup and the link (B)

- `pf pinfold doctor` before setup printed `no-config`. No link
  finding appeared, because no project was loaded.
- `pf pinfold start`, answering `disk` and `y`, exited 0. The box
  started from the existing `profile-default` image, so no build ran.
  `R/bin` then held `git-remote-pinfold -> pinfold`, a relative link.
- Doctor, run against that disk project:
  - with the link: `No issues found.`
  - with the link moved away: `- Git remote: no git-remote-pinfold on
    PATH; run `ln -sf R/bin/pinfold R/bin/git-remote-pinfold``. The
    same text appeared in `--json` `findings`.
  - with `git-remote-pinfold -> /usr/bin/true`: `- Git remote:
    R/bin/git-remote-pinfold is not this pinfold (R/bin/pinfold)`.
- Each case below was a new repo's first `start`. The link was moved
  away first, and every box was stopped afterwards.
  - Share, answering `share`: no link was created. Doctor in that share
    project, with no link anywhere on PATH: `No issues found.`
  - Disk, with a non-executable regular file at `R/bin/git-remote-pinfold`:
    - the file's sha1 was the same before and after (`2624e6b…`);
    - setup printed `Git needs git-remote-pinfold on PATH for this disk
      project: ln -sf R/bin/pinfold R/bin/git-remote-pinfold`.
  - Disk, with `git-remote-pinfold -> /usr/bin/true`: the link was left
    as it was, and no line was printed.
  - Disk, with `chmod 555 R/bin`:
    - no link was created;
    - setup printed the same `Git needs …` line;
    - the mode was then restored.
- `pinfold update` was checked by code reading only. It needs the
  network and replaces the executable it runs as. After a successful
  install it calls the same function with the path it installed to. It
  does not use `current_exe` there, because on Linux that path reads
  `… (deleted)` once the binary is replaced. "Up to date" and failed
  installs create nothing.

### Tags into the box (A)

These ran after `git remote add box pinfold::R/repo`, with the helper
found only in `R/bin`.

- `pf git -c push.followTags=true push box main` exited 0:
  `* [new branch] main -> main`, `* [new tag] v1 -> v1`,
  `* [new tag] v2 -> v2`. The box then held `host/main`, plus `v1` and
  `v2` as tag objects at the host's ids. `v3-topic` and `light` were
  not sent.
- `pf git push box --tags` exited 0 for all four tags. In the box,
  `light` was a commit and the other three tag objects.
- Repeating the followTags push exited 0. Identical tags were accepted.
- Conflicts are recorded under "Final behaviour" below. These first runs
  used the vetoed per-ref version.
- Odd host tags. `pf git push box --tags` exited 0 for each of these,
  and each landed with the host's object id and type:
  - `blobtag`, at a blob;
  - `treetag`, at a tree;
  - `nested`, an annotated tag of annotated `v1`.
- A branch and a tag both named `v1`, pushed as `pf git push box
  refs/heads/v1:refs/heads/v1 refs/tags/v1:refs/tags/v1`, exited 0. The
  box held `host/v1` at the branch's commit and `refs/tags/v1` at the
  tag object.
- Refused, each with exit 1 and `(only a branch to a branch or a tag to
  a tag can be pushed)`:
  - `main:refs/tags/fromtag`;
  - `v1:refs/heads/fromtag`;
  - `:refs/tags/v1`;
  - `main:refs/notes/x`.
- `refs/tags/v1:refs/tags/-x` was accepted and landed as
  `refs/tags/-x`. `check-ref-format` allows that name, as it allows
  `refs/heads/-x`. Every name reaches the guest command line only as a
  full `refs/…` name, never as an argument that starts with `-`.
- In the box, after `git switch -c main host/main`, both `git describe`
  and `git describe --tags` printed `v2-1-gc601e64`.
- Later owner: after `pf pinfold stop`, a transient box listed `v1` and
  `v2` at the same ids.

### With the for-push listing (A, final)

These ran on fresh disk projects with the final build. The first repo
had `main`, annotated `v1` and `v2`, and lightweight `light`.

- First `pf git -c push.followTags=true push box main`, into a box
  without a repository: exit 0. It printed `* [new branch] main ->
  main`, `* [new tag] v1 -> v1` and `* [new tag] v2 -> v2`.
- The second and third identical pushes each printed `Everything
  up-to-date` and exited 0.
- After a new host commit, the push printed only
  `afc7201..f0856f0  main -> main`.
- After a new annotated tag `v3`, it printed only
  `* [new tag] v3 -> v3`.
- The box then created a commit with `commit-tree` (`4c11fa7`) and an
  annotated tag `boxonly` on it (`a5a5011`).
  - The followTags push printed `Everything up-to-date`, exit 0.
  - `git push box --tags` printed only `* [new tag] light -> light`.
  - The host's `git for-each-ref` output and its list of files under
    `.git/objects` were identical before and after.
  - `git cat-file -t` found neither box-only id on the host.
- The box re-tagged `v2` at `v1`, and the host committed:
  - `pf git -c push.followTags=true push box main` exited 0 and printed
    only `f0856f0..468eb38  main -> main`. The box kept its own `v2`,
    because followTags sends only tags the box lacks.
  - `pf git push --force --follow-tags box main` printed `Everything
    up-to-date`, for the same reason.
- In a second repo, with the box's `v2` changed:
  - `pf git push box main v2` exited 1. It printed `c773b7b..12cdb7b
    main -> main` and `! [rejected] v2 -> v2 (already exists)`. The
    branch landed and the box's `v2` was unchanged.
  - `pf git push box +refs/tags/v2:refs/tags/v2` exited 0 with
    `+ 7bb473a...c8f5d5e v2 -> v2 (forced update)`.
  - After the box changed `v2` again, `pf git push --force box main v2`
    exited 0 and replaced it.
- Host rewrite of `main` (reset, then a new commit):
  - `pf git push box main` exited 1 with `! [rejected] main -> main
    (non-fast-forward)`.
  - `pf git push --force box main` exited 0 with `+ 468eb38...09b750a
    main -> main (forced update)`.
  - Before this change, branches were always forced.
- Fetch direction, after the box made a branch `work`, a tag `boxwork`
  and an annotated tag `main`:
  - `pf git fetch box` printed `* [new branch] work -> box/work`, exit 0.
  - `pf git fetch --tags box` printed nothing, exit 0.
  - `pf git ls-remote box` listed only `refs/heads/work`.
  - The host's `refs/tags` were identical before and after.
- Timing, wall clock for `git push box main`, three runs each:

  | Case | Before | After |
  | --- | --- | --- |
  | Owner running, nothing new | 0.70–1.63 s, a full push | 1.72–1.84 s, listing only, `Everything up-to-date` |
  | Owner running, new commit | — | 2.85 s, listing plus push |
  | No owner, nothing new | 2.76–3.22 s, a full push | 3.47–3.49 s, one transient box |
  | No owner, new commit | — | 6.09 s, two transient boxes |

  So a push that sends something costs one extra box exec: about 1 to
  2 s with a running owner, and about 3 s without one, where the
  listing starts its own transient box.

### Exit-65 behaviour (A, before the listing)

These ran before `list for-push` listed anything, so Git attempted every
ref. The repo had
`main` (three commits), annotated `v1` and `v2`, and lightweight
`light`.

- `pf git -c push.followTags=true push box main` exited 0. It printed
  `* [new branch] main -> main`, `* [new tag] v1 -> v1` and
  `* [new tag] v2 -> v2`.
- Running the same push again printed the same three lines and exited 0.
  It was not quiet; the listing above fixed that.
- The box installed a `reference-transaction` hook that echoes
  `hook-stdout`; run by hand, it printed that. The box also re-tagged
  `v2` at `v1`. The host then committed `00b3f87`.
- Conflict without force: `pf git -c push.followTags=true push box main`
  exited 1. It printed
  `pinfold: a pushed tag already exists in the box with a different
  target; nothing was pushed (use --force to replace box tags)`, then
  `! [remote rejected] main -> main (…same reason…)` and the same for
  `v1` and `v2`. The box's `host/main` stayed at `12ebe40`, and its
  `v2` stayed at the box's `00882e8`.
- `pf git push --force --follow-tags box main` exited 0. The box's
  `host/main` moved to `00b3f87`, and its `v2` became the host's
  `61c8ab0`.
- The box re-tagged `v2` and `light` again. The hook printed
  `hook-stdout` twice.
  - `pf git push box +refs/tags/v2:refs/tags/v2` exited 0, and `v2` was
    back at `61c8ab0`.
  - `pf git push box refs/tags/light:refs/tags/light` exited 1 with the
    same reason.
  - `pf git push box +refs/tags/light:refs/tags/light` exited 0.
- Branches were then always forced. After `side` was moved back to
  `main~2`, `pf git push box side` without `--force` exited 0, and the
  box's `host/side` followed it back to `3798e89`.
- The hook was present for every push after it was installed. No
  outcome changed.
- A tag that appears between the check and the apply is covered by the
  plain-git probe in Design. The race could not be timed through the
  CLI.

### Box tags stay in the box (A)

The host pushed a tag named `feature`. In the box, a commit was then
made on a new branch `feature`. The box then created:

- `boxtag`, lightweight;
- `main`, an annotated tag named like a branch, at `feature~1`;
- `feature2`, an annotated tag, plus a branch `feature2` at a different
  commit;
- `boxblob`, at a blob written only in the box;
- `boxtree`, at a tree;
- `boxnested` and `boxnested2`, nested annotated tags;
- `tip`, at a new commit.

Then:

- `pf git fetch box` exited 0. It updated `box/feature` to the box's
  branch commit, not the tag's, and added `box/feature2` at the branch's
  commit, not the tag's.
- `pf git fetch --tags box` exited 0 and printed nothing.
- `pf git ls-remote box` listed only `refs/heads/feature`, `feature2`
  and `main`. `pf git ls-remote --tags box` printed nothing.
- Host `refs/tags`, compared by `git for-each-ref`, was byte-identical
  before and after (sha1 `ab18b45…` both times). The only change to all
  refs was under `refs/remotes/box/`.
- `git cat-file -t` on the host found none of the box-only objects: the
  blob, the tag objects of `main`, `feature2`, `boxnested`,
  `boxnested2` and `tip`.

An earlier run included `git -c fetch.prune=true -c
fetch.pruneTags=true fetch box`. It deleted every host tag. This is the
documented "Not protected" case: the box lists no tags. It was there
before this change, and the change leaves it as it was.

### Review fixes

These were verified on a fresh disk project with the build of that
commit.

- **Inner fetch exit status.** The box's fetch now also passes:
  - `--no-prune` and `--no-prune-tags`, so a box `fetch.prune` cannot
    delete other `host/*` refs;
  - `--no-recurse-submodules`;
  - `--no-auto-maintenance`;
  - `--no-write-commit-graph`.

  So the fetch's exit status reflects only its ref transaction.
  - A plain-git 2.56.1 probe used a clone with
    `fetch.recurseSubmodules=true` and a submodule whose remote was
    gone. With the old flags, the fetch exited 1 after `host/main` had
    landed (`Errors during submodule fetch: sub`). With the new flags
    it exited 0.
  - Through the binary, the box repository was set up the same way:
    `git submodule add /tmp/sub sub`, `fetch.recurseSubmodules true`,
    then `/tmp/sub` removed. `pf git push box main` exited 0 with
    `004572a..b0fe1db  main -> main`, and the box's `host/main` was
    `b0fe1db`.
- **Listing fallback.** Checked by code reading only; I could not end
  the box mid-listing on cue. `launch::cross` returns an outer
  `io::Error` when the box ends during the work, the check refuses, the
  disk is missing or a signal arrives. `list for-push` now returns
  early only for `ErrorKind::Interrupted` and otherwise lists nothing.
- **Link remedy.** It now prints `ln -s` without `-f`, and names a
  taken path instead of printing a command. With an empty directory at
  `R/bin/git-remote-pinfold`:
  - Setup printed `Git needs git-remote-pinfold on PATH for this disk
    project: R/bin/git-remote-pinfold is taken by something other than
    pinfold`. The directory stayed empty.
  - Doctor printed `- Git remote: no git-remote-pinfold on PATH;
    R/bin/git-remote-pinfold is taken by something other than pinfold`.
  - With nothing at the path, doctor printed `- Git remote: no
    git-remote-pinfold on PATH; run `ln -s R/bin/pinfold
    R/bin/git-remote-pinfold``.
  - A link to pinfold whose directory is not on PATH prints `… links to
    pinfold but its directory is not on PATH`. That case was not run.
- The earlier texts above that show `ln -sf` are from before this fix.

### Review fixes, round 2

- **Lazy fetch from the listing.** In a partial clone, Git looks up the
  listing's ids while it plans a push, and a missing id triggers a
  fetch from the promisor remote.
  - Setup:
    - `origin` was a local repo with `uploadpack.allowFilter`.
    - The project was `git clone --filter=blob:none file://…/origin`.
    - `remote.origin.uploadpack` pointed at a wrapper that logs each
      run.
    - The box tagged a box-only commit (`213b32d…`) twice, as an
      annotated `boxonly` and a lightweight `boxcommit`.
  - With the previous binary (6c71aae), the followTags push ran
    upload-pack twice (the log went from 1 to 3). Git printed `fatal:
    git upload-pack: not our ref 213b32da2ec7d5edbd73907da18eb1bd3512cd3b`
    and the same for the tag's id. `GIT_TRACE` showed `git -c
    fetch.negotiationAlgorithm=noop fetch origin … --filter=blob:none
    --stdin`.
  - With the fix, the same push ran no upload-pack (the log stayed at 1,
    the clone).
- **Round 2's fix, since replaced.** Round 2 kept ids found by `cat-file
  --batch-check` with `GIT_NO_LAZY_FETCH=1`. A repository with a
  promisor remote got an empty listing, because Git's release notes put
  the variable in 2.45.0. Round 3 replaced it; see Design and the
  section below.
- **No disk writes from the listing.** The receiver's bundle file is
  now optional. The for-push listing passes none, so it creates no
  bundle file and fails on the first byte after the empty line. Fetch
  still passes its bundle and is unchanged. Checked by code reading:
  the listing script is pinfold's own, so a box cannot easily make it
  emit trailing bytes.
- **Control-character install path.** The binary was copied into a
  directory named `bin<U+0001>x`, and doctor ran with no helper on PATH.
  It printed `- Git remote: no git-remote-pinfold on PATH; link
  git-remote-pinfold to pinfold in a directory on PATH`, with no command.

### Review fixes, round 3

These ran through a build of the round-3 commit.

- **Listing kept to host ref tips.** In a full clone with `main`, `v1`
  and `v2`:
  - the first followTags push printed `* [new branch] main -> main`,
    `* [new tag] v1 -> v1` and `* [new tag] v2 -> v2`;
  - the second and third each printed `Everything up-to-date`, exit 0.
- After a new host commit, the push printed `* [new branch] main ->
  main`, exit 0, not a range. The box's `host/main` sat at the old
  commit, which is no longer a host ref tip, so it was dropped from the
  listing. The next push printed `Everything up-to-date`.
- The box re-tagged `v1` at a box-only commit (`8441d5d`), and the host
  committed again. A followTags push exited 1. It printed `pinfold: a
  pushed tag already exists in the box with a different target; nothing
  was pushed (use --force to replace box tags)`, then `! [remote
  rejected]` with that reason for `main` and `v1`. The box's
  `host/main` and `v1` were unchanged.
- `git push --force --follow-tags box main` exited 0 with `* [new
  branch] main -> main` and `* [new tag] v1 -> v1`. The box then held
  the host's `main` (`f9e3947`) and `v1` (`2c072a0`). The next
  followTags push printed `Everything up-to-date`.
- **Partial clone**, set up as in round 2 (upload-pack wrapper, log at 1
  after the clone):
  - after the first push, the box made a box-only commit `b93535c…`,
    tagged it (`boxcommit`, annotated `boxonly`) and moved its
    `host/main` there;
  - the next followTags push printed `* [new branch] main -> main`, exit
    0, and the one after printed `Everything up-to-date`;
  - the log stayed at 1, so no upload-pack ran.
- **Non-UTF-8 install path.** The remedy builds a command only from
  `Path::to_str`; otherwise it prints the generic sentence. This was
  checked by code reading only, because the macOS filesystem refused
  `mkdir` of a name holding byte 0xFF.

### Checks

- `cargo fmt --check` passed.
- `cargo clippy --all-targets --locked -- -D warnings` passed.
- `RUSTC_WRAPPER= cargo clippy --target x86_64-unknown-linux-musl
  --all-targets --locked -- -D warnings` passed. `CC` was a wrapper that
  runs `zig cc -target x86_64-linux-musl` and drops cc-rs's
  `--target=` argument.
- No Linux box run.

### Cleanup

- Every box started was stopped.
- Twenty-five disks were removed by name: `pi-repo-8800e3f439bd-{home,project}` (seven times), `pi-full-81c7a40f3262-{home,project}`, `pi-partial-96d3ee279c8a-{home,project}`,
  `pi-r-{file,link,ro}-…-{home,project}` and
  `pi-r-share-7e8205bba800-home`.
- `R` was removed.
- No image was built or removed.
