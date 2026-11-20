# `git fetch box` brings the box's tags (2026-11-20)

The owner decided that `git fetch box` brings the box repository's tags
back to the host as real tags. Until now no tag crossed in that
direction. He accepted that `push.followTags` may later publish such a
tag. A host tag is never changed or deleted by a fetch.

The test suite is suspended. Everything below ran through a debug build
of the branch tip on macOS: Apple container 1.7.1, host git 2.56.1, box
git 2.47.3.

## Design

- The box script lists `refs/heads/*` and the `refs/tags/*` whose
  `%(objecttype)%(*objecttype)` is `commit` or `tagcommit`. It bundles
  them with the branches.
  - Box git 2.47.3 peels `%(*objecttype)` fully. A nested annotated tag
    of a commit is therefore listed too, at no extra code.
  - A tag at a blob or tree is neither listed nor bundled.
- `bundle create` always writes an annotated tag's object as a head,
  even when its commit is held. A lightweight tag at a held commit is
  dropped, as a branch is.
  - The "anything new" probe therefore also bundles when an annotated
    tag is listed.
  - A plain-git probe: the revs `refs/tags/at refs/tags/lt ^C` gave a
    bundle with only `refs/tags/at`, and `rev-list -n1` printed nothing.
- The listing goes through the same `receive`, with the allowed prefixes
  `refs/heads/` and `refs/tags/`. The cap of 4096 now counts refs.
- On the host, each box tag is handled as follows:
  - **Host has the name.** A box tag whose name matches a host tag's,
    ignoring case, is never listed. Neither is a non-ASCII name. The
    host's tag is listed at its own id.
  - **Name check.** A new name must pass `check-ref-format` as itself,
    or it is left out.
  - **Lightweight tag.** It must meet the branch rule: a commit the
    box's pack carries as a bundle head, or one that
    `refs/remotes/box/*` reaches.
  - **Annotated tag.** It must be a bundle head that the box's pack
    carries. `cat-file --batch-check` of `OID^{}`, run through the
    quarantine, must name a commit that the pack carries or `box/*`
    reaches.
  - **Failures.** A bundle head that fails fails the fetch. A tag
    outside the bundle that fails is left out.
- Every host tag is listed back at its own id, so Git finds nothing to
  prune and nothing to move, whatever the flags or config. Refusing
  instead could not work. The helper cannot see `--prune-tags` or
  `--force`, because Git passes no options to a helper without the
  `option` capability.
- The host reads its tags after the box's stream. A slow box therefore
  does not widen the window for a tag the host creates during the fetch.
- Each new annotated tag is listed with `COMMIT refs/tags/NAME^{}`. A
  fake helper over plain git showed this is needed.
  - Without the peeled line, an annotated tag on an already-fetched
    commit was not followed, and Git sent no `fetch`.
  - With it, Git sent `fetch OID refs/tags/t1` and printed
    `* [new tag] t1 -> t1`.
- Fix found on the way: a listing over the ref cap closed the pipe, and
  the box's git then failed on it. The fetch reported `the box's git
  failed`. The listing's own error now wins.

### Why case is folded

Loose refs are files. On a case-insensitive volume (an APFS disk image
mounted under `target/`, where `core.ignorecase=true`), plain git 2.56.1
was run with host tag `Rel` at A and a source repository with tag `rel`
at B:

- **`Rel` loose.** `git fetch --no-prune ../d
  refs/tags/rel:refs/tags/rel` printed `* [new tag] rel -> rel`, and
  the host's `Rel` then pointed at B. The same happened with `+`.
- **`Rel` packed.** The same fetch created a loose `rel`, and
  `git rev-parse refs/tags/Rel` then gave B.

An exact name check would have let a box tag move a host tag. APFS also
ignores Unicode normalization. pinfold has no normalization code, so
non-ASCII box tag names are left out. Review fix 2 below closes the
remaining host-side case.

## Setup

- `R` = `target/fbt` under the worktree. It held:
  - `R/bin/pinfold`, the debug binary;
  - `R/bin/git-remote-pinfold -> pinfold`, created by setup;
  - `R/xdg/{config,state,cache}`.
- `W` = `target/ci`, a 200 MB case-insensitive APFS image mounted
  there with `hdiutil`. `W/repo` was the project.
- `R/pf` set those XDG roots, `PINFOLD_PROFILE=default`,
  `VISUAL=true EDITOR=true`, `PINFOLD_NO_UPDATE_CHECK=1` and
  `PATH=R/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin`.
- HOME was not changed, so the operator's global git config applied.
  It sets `fetch.prune=true` and `push.followTags=true`. Every plain
  `git fetch box` below therefore also pruned `box/*`. The test repo
  set `tag.gpgsign=false`.
- `expect` answered `disk` and `y` to `pf pinfold start`. The box ran
  from the existing `profile-default` image. No image was built.
- The host repo held:
  - `main`, with commits `one` and `two`;
  - annotated `v1`, and lightweight `hostlight` and `Rel`, on `one`;
  - annotated `hostonly` and lightweight `same`, on `two`.
- `git remote add box pinfold::W/repo`, then
  `git -c push.followTags=true push box main`, exit 0. It printed
  `* [new branch] main -> main`, `* [new tag] hostonly -> hostonly` and
  `* [new tag] v1 -> v1`. The second push printed
  `Everything up-to-date`.

An earlier run on a case-sensitive directory, with the first build,
gave the same results for every case it shared with this one.

## Cases

**New commit with an annotated and a lightweight tag.** The box ran
`git switch -c main host/main`, committed `three` (`5e46f71`), then ran
`git tag -a boxann` and `git tag boxlight`.

- `git fetch box`, exit 0:

  ```
  From pinfold::W/repo
     c2f53ee..5e46f71  main       -> box/main
   * [new tag]         boxann     -> boxann
   * [new tag]         boxlight   -> boxlight
  ```

  Git brought both tags by auto-follow, since both point at the fetched
  commit. The host's `refs/tags` gained only these two.
- A repeat `git fetch box`, and `git fetch --tags box`, printed nothing
  and exited 0.
- `git ls-remote box` listed `refs/heads/main`, then `Rel`, `boxann`,
  `boxlight`, `hostlight`, `hostonly`, `same` and `v1`, each at the
  host's id. Host tags appear there because they are listed back.

**Annotated tag on an already-fetched commit, nothing else new.** The box
tagged `HEAD~2` as annotated `onold`. `git fetch box` printed
`* [new tag] onold -> onold`.

**Tags no branch reaches.** The box made two commits with `commit-tree`,
on no branch, and tagged them annotated `detached` and lightweight
`detlight`.

- `git fetch box` printed nothing and brought neither.
- `git fetch --tags box` printed `* [new tag] detached -> detached` and
  `* [new tag] detlight -> detlight`.

**Same name, other object; case variants; non-ASCII.** The box changed
three tags and added three new ones:

- re-tagged `v1` (annotated, at `5e46f71`) and `same` (at `5e46f71`);
- re-tagged `hostlight` as annotated;
- added lightweight `rel` (host has `Rel`);
- added annotated `HOSTONLY` (host has `hostonly`);
- added lightweight `café`.

Each of these exited 0, printed nothing, and left
`git for-each-ref refs/tags` identical:

- `git fetch box`;
- `git fetch --tags box`;
- `git fetch --force --tags box`;
- `git fetch box '+refs/tags/*:refs/tags/*'`. With the global
  `fetch.prune`, this is also a pruning fetch.
- `git fetch --force box refs/tags/v1:refs/tags/v1`.

Two further checks:

- `git fetch --force box refs/tags/rel:refs/tags/rel` exited 128 with
  `fatal: couldn't find remote ref refs/tags/rel`. `refs/tags/Rel`
  stayed at `3e0631b`.
- `git ls-remote --tags box` listed `v1`, `same`, `hostlight`,
  `hostonly` and `Rel` at the host's ids. It listed no `rel`, no
  `HOSTONLY` and no `café`.

In the first run, the box's tag objects for the re-tagged `v1` and
`hostlight` were absent from the host store after these fetches,
because none of them landed anything. After the next fetch that landed
a ref, both were in the host store, unreferenced. The quarantine moves
in whole.

**Identical tags.** `v1` and `hostonly` were identical on both sides
until the box re-tagged `v1`. They gave no change and no error in every
fetch.

**Odd tags.** The box wrote a blob only it held and made a tree of it
with `mktree`. It then created:

- `blobtag`, lightweight, at the blob;
- `treetag`, annotated, at the tree;
- `nested`, annotated, of `boxann`;
- `main`, annotated, named like the branch, at `HEAD~1`.

Results:

- `git fetch box` printed `* [new tag] main -> main` and
  `* [new tag] nested -> nested`.
- `git fetch --tags box` printed nothing more. The host's `refs/tags`
  gained only `main` and `nested`.
- `git ls-remote --tags box` listed neither `blobtag` nor `treetag`.
- On the host, `git cat-file -t` found none of the blob, the tree or
  the `treetag` object. It found `nested` and `main` as tags.
- With the tag `main` present, `git -c push.followTags=true push box
  main` failed with `error: src refspec main matches more than one`,
  exit 1. Pushing `refs/heads/main` worked. After `git tag -d main`,
  the short name worked again. The spec and README now say this.

**Pruning.** The host added lightweight `hostprune1` and annotated
`hostprune2`, which the box never had. The box deleted `detlight`,
which the host had fetched, and added lightweight `pruneprobe`. Each
fetch below was compared against `git for-each-ref refs/tags` from just
before it:

- `git -c fetch.prune=true -c fetch.pruneTags=true fetch box`, exit 0,
  printed `* [new tag] pruneprobe -> pruneprobe`. The only difference
  was the added `pruneprobe`. Every host tag survived, `detlight`
  included.
- `git fetch --prune --prune-tags box`: nothing printed, unchanged.
- `git fetch --prune --tags box`: nothing printed, unchanged.
- `git -c remote.box.pruneTags=true fetch --prune box`: nothing
  printed, unchanged.

The 2026-11-19 record saw `fetch.pruneTags` with prune delete every host
tag. That no longer happens.

**Push still works.** The host fast-forwarded to `box/main` and
committed `four`.

- `git -c push.followTags=true push box main`: the `main` ambiguity
  above.
- `git -c push.followTags=true push box refs/heads/main` exited 1. It
  printed `pinfold: a pushed tag already exists in the box with a
  different target; nothing was pushed (use --force to replace box
  tags)`, with `! [remote rejected]` for `main`, `hostprune2` and `v1`.
  The cause was the box's re-tagged `v1`. This is the push rule from
  #365, unchanged.
- After `main` was deleted on both sides and the box deleted `v1`:
  - `git -c push.followTags=true push box main` exited 0. It printed
    `c2f53ee..52924c5  main -> main`, `* [new tag] hostprune2 ->
    hostprune2` and `* [new tag] v1 -> v1`.
  - The same push again printed `Everything up-to-date`, exit 0.
  - The box held `host/main` at `52924c5` and `v1` at the host's
    `03d73b2`.

**Ref cap.** This ran in the first run, on a case-sensitive directory.
The box created 4100 lightweight tags with `update-ref`.

- `git fetch box` printed `pinfold remote: the box lists more than 4096
  refs`, exit 128. Before the fix above, it printed `pinfold remote: the
  box's git failed`.
- After the box deleted them, `git fetch box` exited 0.

**End state.**

- `git fetch box` printed `+ 52924c5...5e46f71 main -> box/main (forced
  update)`. Push moves `box/main` until the next fetch, as before.
- Two more fetches, with and without `--tags`, printed nothing.
- `git fsck` exited 0.
- No `pinfold-incoming-*` directory or bundle file was left.

## Limits

- **Ref count.** The 4096 cap now counts branches and tags. A box
  repository with more than 4096 tags, such as a clone of a large
  project, fails every fetch, branches included. Not changed here.
- **Repeat cost.** The host sends only `box/*` tips as held. Every fetch
  therefore re-bundles every annotated tag object the box lists, and the
  commits that only tags reach.
  - In the first run, the box script was run by hand after `detached`
    and `detlight` had landed. It gave 1211 bundle bytes: prerequisite
    `cc96d08`, and heads `boxann`, `detached`, `detlight`, `hostonly`,
    `onold` and `v1`.
  - The host indexes that bundle and drops it, because Git asks for
    nothing.
  - Sending host tag ids as held would avoid this, but would tell the
    box which tags the host has. Not changed here.
- **Size.** The 4 GiB bundle limit is unchanged. Tag objects are small,
  but a tag on a large history that no branch reaches adds that history.
- **Lookups.** The host runs `cat-file` on box-supplied ids, as it
  already did for branches. In a partial clone, that can reach the
  promisor remote. Tags add more such ids. Not changed here.

## Not run

- Hostile tag cases: a bundled tag at a blob, a bundled tag peeling to
  a commit `box/*` does not reach, and an invalid tag name as a bundle
  head. They need a hostile box image, and this change builds no image.
  Checked by code reading: each is a bundle head that fails the check
  above, so the fetch fails before Git sees a ref.
- No Linux box run.

## Checks

- `cargo fmt --check` passed.
- `cargo clippy --all-targets --locked -- -D warnings` passed.
- `RUSTC_WRAPPER= cargo clippy --target x86_64-unknown-linux-musl
  --all-targets --locked -- -D warnings` passed. `CC` was a wrapper that
  runs `zig cc -target x86_64-linux-musl` and drops cc-rs's `--target=`.

## Cleanup

- Both boxes were stopped.
- Four disks were removed by name: `pi-repo-27b211e1ba26-{home,project}`
  and `pi-repo-0d7de29db0cd-{home,project}`.
- The disk image was detached. `R`, `W` and the probe repositories
  under `target/` were removed.
- No image was built or removed.

## Review fixes

These are review findings on `ff2d2a9`. They were checked through a
debug build of the fix commit, on a case-sensitive directory under
`target/`, with the same harness and `tag.gpgSign=false`.

1. **Non-UTF-8 host tag names.** The host read its tags with a lossy
   conversion. A name that is not UTF-8 would then be listed back under
   another name, and `--prune --prune-tags` could delete the real one.
   - The host now splits the raw bytes and refuses the fetch as `spec`
     if any host tag's name is not UTF-8. No listing is produced.
   - Checked through the binary. APFS refuses such a file name, so the
     name was appended to `.git/packed-refs` as
     `OID refs/tags/bad\377`.
   - `git fetch box` and `git fetch --prune --prune-tags box` each
     exited 128 with `pinfold remote: spec: a host tag's name is not
     UTF-8, so a fetch from the box could delete it; rename it (`git
     for-each-ref refs/tags` shows it)`. Host tags were unchanged.
   - After the line was removed, `git fetch box` exited 0.
2. **Unicode normalization.** While any host tag's name is non-ASCII,
   no box tag is listed. Host tags are still listed back, and branches
   are unaffected. Case is compared in ASCII only.
   - The host created tag `réf`. The box then committed and tagged
     `newtag` (annotated) and `newlight`.
   - `git fetch box` printed only `f96fdf0..3e52d7d main -> box/main`.
   - `git fetch --tags box` printed nothing.
   - `git ls-remote --tags box` listed only host tags, `réf` included.
   - `-c fetch.prune=true -c fetch.pruneTags=true fetch box` left every
     host tag in place.
   - After the host deleted `réf`, `git fetch box` printed
     `* [new tag] newlight -> newlight` and
     `* [new tag] newtag -> newtag`.
3. **Non-ASCII box tags.** The box script drops such tags, so one bad
   name no longer aborts the fetch. Under `LC_ALL=C`, it drops any tag
   line with a byte outside printable ASCII. The host-side check stays
   as the backstop.
   - The box made `boxann`, `boxlight`, `café` and the symbolic tag
     below on a new commit.
   - `git fetch box` printed `* [new tag] boxann -> boxann` and
     `* [new tag] boxlight -> boxlight`, exit 0.
   - `git ls-remote box` listed no `café`.
4. **Symbolic tags.** The box's `for-each-ref` format now puts
   `%(symref)` before the type field, so a symbolic tag never matches
   the filter.
   - The box ran `git symbolic-ref refs/tags/sym refs/heads/main`.
   - `sym` was not listed by `git ls-remote box` and did not land.
5. **Duplicates.** The duplicate check now runs on every listed name,
   before the name is validated. This was checked by code reading only:
   the box's own script cannot list a ref twice.
6. **Docs.**
   - Any command-line refspec that maps a ref to another tag name is
     named as able to change or delete host tags.
   - The claims that tags no branch reaches come only with `--tags` are
     removed. Through the binary, the box made an annotated `detached`
     on a commit no branch reaches, then committed on `main`. A plain
     `git fetch box` printed `c0fcf52..f96fdf0 main -> box/main` and
     `* [new tag] detached -> detached`. The whole bundle migrates, and
     Git follows the tag in its second pass. The earlier "Tags no branch
     reaches" case brought nothing only because no branch moved.
   - Nested tags are said to cross only where the box's Git peels
     `%(*objecttype)` fully.
   - Row 10's bundled-tag failure now reads: the peeled commit is
     neither carried by the box's pack nor reachable from `box/*`.

Other checks in the same run:

- **Collisions.** The box re-tagged `v1` and added `HOSTLIGHT`. Host
  tags were unchanged after each of these:
  - `git fetch --force --tags box`;
  - `git fetch box '+refs/tags/*:refs/tags/*'`;
  - `-c fetch.prune=true -c fetch.pruneTags=true fetch box`;
  - `git fetch --prune --prune-tags box`.
- **Push.** After the box deleted its `v1`:
  - `git -c push.followTags=true push box main` printed
    `ec4c436..3e52d7d main -> main` and `* [new tag] v1 -> v1`;
  - the next two printed `Everything up-to-date`.
- **End state.** `git fetch box` printed nothing. `git fsck` exited 0.
  No quarantine or bundle file was left.

Declined in review and unchanged here: `index-pack --fsck-objects`
admitting a pack with broken links outside the advertised history, and
`git bundle verify` running outside the quarantine. Both steps predate
this change. Tag bundles pass through them unchanged.

- `cargo fmt --check`, `cargo clippy` and the Linux-target clippy
  passed.
- The box was stopped, and `pi-repo-27b211e1ba26-{home,project}` were
  removed by name.
- `R` was removed. No image was built or removed.
