# s280 Linux acceptance check (dev VM), 2026-11-17

Commit tested: `6444bd3d7ce9927af61994da6576b1d2f0be34d7` (branch `share-project-280`, PR #330),
`pinfold 0.5.5`, debug build `cargo build --locked --target x86_64-unknown-linux-musl`
(incremental, 13 s). Host: dev VM, Debian 13, kernel 6.12, user in `kvm`.

## Setup

Private roots under `~/probe280`:

```sh
export XDG_CONFIG_HOME=~/probe280/config XDG_STATE_HOME=~/probe280/state \
       XDG_CACHE_HOME=~/probe280/cache XDG_DATA_HOME=~/probe280/data
export PINFOLD_NO_UPDATE_CHECK=1 PINFOLD_CPUS=2 PINFOLD_MEMORY=1G EDITOR=true VISUAL=true
PF=~/pinfold/target/x86_64-unknown-linux-musl/debug/pinfold
cd ~/probe280/proj && git init -b main && (local user.name/email) && commit "first commit"  # 85d20bb
$PF config edit
```

`config edit` wrote `mounts = [{ host = ".", kind = "disk" }]`. `$PF config` reported
`"mount":{...,"kind":"disk"}`, `"disks":{"home":"pi-proj-…-home","project":"pi-proj-…-project"}`,
`image_built:false`.

## Share refused as spec (Linux)

Config replaced with `mounts = [{ host = ".", kind = "share" }]`, then `$PF start`:

```
pinfold start: …/pinfold.toml: spec: share mounts are unsupported on Linux
start rc=125
```

No firecracker process, and `/tmp/pinfold-<uid>` did not exist afterwards. Config restored to `disk`.

## Image build

`$PF build` (profile default, BuildKit/Firecracker builder): 145 s from a cold private cache.
Output ended `localhost/pinfold/profile-default:latest`.

## (a) Detached owner: start, two joins, stop — PASS

```
$PF start                                   -> rc=0, one firecracker process
$PF -- sh -c "cat /proc/sys/kernel/random/boot_id; echo marker-from-cmd1 > /tmp/join-marker; git log --oneline; git branch -a"
   d3ca038a-…   85d20bb first commit   * main   remotes/host/main   rc=0
$PF -- sh -c "cat /proc/sys/kernel/random/boot_id; cat /tmp/join-marker"
   d3ca038a-…   marker-from-cmd1   rc=0
pgrep -c firecracker -> 1
```

Concurrent joins: one client `sh -c "boot_id; sleep 4; echo long-done"` in the background,
a second `sh -c "boot_id; exit 7"` started 1 s later. Both printed boot id `d3ca038a-…`;
second returned rc=7 (exit code passed through) while the first kept running and ended
`long-done`, rc=0.

`$PF stop` -> rc=0, no stdout, `pgrep -c firecracker` 0.

Also: `$PF -- true` with no owner -> rc=0, no firecracker left afterwards.

## (b) Startup fast-forward — PASS

While the owner ran, host commit `6e68e1b second commit`. A join then showed
`$PF -- git log --oneline main host/main` -> only `85d20bb first commit` (joining runs no sync,
per spec).

```
$PF stop   -> rc=0, no firecracker
$PF start  -> rc=0
$PF -- sh -c "cat /proc/sys/kernel/random/boot_id; git log --oneline; git status --short; ls"
   97a94d3d-… (new box)   6e68e1b second commit   85d20bb first commit   (clean)   a.txt b.txt
```

The new owner pushed and fast-forwarded the clean, strictly-behind `main`, and the working tree
has the new file.

Not-fast-forward case (diverged): guest commit `713b96b guest-commit`; host commit
`ae9d126 third commit`; `$PF stop; $PF start`; `$PF -- git log --oneline -3` ->
`713b96b guest-commit / 6e68e1b / 85d20bb`. Checkout left as the box had it, as spec says.

## fetch / push

With owner running:

```
$PF fetch -> {"box":"pi-proj-…-79f2d3af57af","event":"fetched","refs":{"refs/remotes/pinfold/main":"713b96b…"},"remote":"pinfold"} rc=0
            host: git log -1 pinfold/main -> 713b96b guest-commit
$PF push  -> {"box":"pi-proj-…-79f2d3af57af","event":"pushed","refs":{"refs/remotes/host/main":"ae9d126…"},"remote":"host"} rc=0
            box: git log -1 host/main -> ae9d126 third commit
```

Both used the running owner's box (same box name). With no owner, `$PF fetch` started a
transient box (`…-cb0e3ed5dd22`), printed `fetched`, rc=0, and left no firecracker process.
`push` without an owner was not run.

## Not established

- `push` with no owner (only `fetch` was exercised without an owner).
- Departure signalling (TERM/KILL of a departed client's group) and `config-changed` refusal
  were outside this check.
- Concurrency evidence is boot id plus a `/tmp` marker; a `ps | grep -c` line in the concurrent
  step counted its own `sh -c` argv and proves nothing, so it is ignored.

## Cleanup

`$PF stop` (rc=0) before cleanup. `rm -rf ~/probe280 /tmp/pinfold-<uid>` (rc=0). The image,
disks, builder cache (4.2 GiB) and run records were all under the private roots. Afterwards
no pinfold, firecracker or buildkit process for the user, no `/tmp/pinfold*`. A `find` for
user-owned files newer than the probe outside `~/probe280` and `~/pinfold` found nothing
before removal. `~/pinfold` is left detached at `6444bd3`, working tree clean, with its
`target/` build output.
