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
- `list for-push` answers with an empty list. Listing box tags there
  would mean reading box data on the host, so Git believes the box has
  no refs. With `push.followTags`, each push therefore resends every
  reachable annotated tag and prints `* [new tag]` for it. Identical
  tags pass the check, so the repeat exits 0, but it is not quiet.
  Making it quiet needs a box listing on the host, which this design
  does not have. The fetch direction is unaffected either way.

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

### Final behaviour (A)

These ran on a fresh disk project with the final build. The repo had
`main` (three commits), annotated `v1` and `v2`, and lightweight
`light`.

- `pf git -c push.followTags=true push box main` exited 0. It printed
  `* [new branch] main -> main`, `* [new tag] v1 -> v1` and
  `* [new tag] v2 -> v2`.
- Running the same push again printed the same three lines and exited 0.
  It was not quiet; see Design.
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
- Branches are always forced. After `side` was moved back to
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
- Thirteen disks were removed by name: `pi-repo-8800e3f439bd-{home,project}` (three times),
  `pi-r-{file,link,ro}-…-{home,project}` and
  `pi-r-share-7e8205bba800-home`.
- `R` was removed.
- No image was built or removed.
