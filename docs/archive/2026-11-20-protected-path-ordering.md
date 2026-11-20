# Protected-path ordering (#357, #358), 2026-11-20

Guarantee 13: a `host-path-in-project` refusal creates no project state or
configuration. Two orderings broke it.

## Fix

- #357: `admit` and `borrow` opened the maintenance and launch locks under
  the lock directory before any protected-path check. They now first run
  `config::check_roots_outside`: the existing check with no profile, one
  direction (a protected path inside the project). The configured check
  under the locks is unchanged, so the new check only adds an earlier
  refusal. It compares paths and holds no state the locks protect.
- #358: on macOS the init directory under `artifacts/` joined the base
  protected set in `mounts::protected_paths`. It was only in
  `mounts::prepare`'s extra set, which runs after first-run setup saved the
  configuration and the owner recorded state. `protected_paths` resolves
  every entry, so `prepare` and the pre-save checks share one resolution.
  Linux is unchanged: its init is pinfold's own executable.

## Runs

macOS, Apple runtime, debug builds of main at 774f964 (`before`) and the
fix (`after`). A wrapper set throwaway `XDG_CONFIG_HOME`, `XDG_STATE_HOME`
and `XDG_CACHE_HOME` under a fresh temp dir, `PINFOLD_PROFILE=default`,
`VISUAL=true EDITOR=true`, and refused a cwd under the user's code tree.
Interactive runs used `expect`, answering the mount question and the
save prompt. The existing `profile-default` image served every run; no
image was built or removed. Lock files were counted with an `ls` diff of
`/private/tmp/pinfold-501` around each run.

### #357

The lock directory is fixed at `canonicalize("/tmp")/pinfold-<uid>`, with
no override. A project that contains it would be `/private/tmp` or an
ancestor, shared with the user's real runs, so the issue's case was not
run. A proxy shows the same ordering: an unconfigured non-Git project
holding its own `XDG_STATE_HOME`, a protected root that also needs no
configuration to be checked.

| Case | before | after |
|---|---|---|
| `-- true`, redirected | `no-config`, exit 125, two new `.lock` files | `host-path-in-project: …/state/pinfold resolves inside …`, exit 125, no new lock |
| `start`, interactive | prompts, then `host-path-in-project … overlaps …` after "Save", exit 125, two new `.lock` files | `host-path-in-project` before any prompt, exit 125, no new lock |
| `git push box main` in a Git repo, unconfigured | `pinfold remote: no-config`, two new `.lock` files, and the `state/` root created inside the project | `pinfold remote: host-path-in-project`, no new lock, project holds only `.git` |

In every after case the throwaway config root holds no file for the
project.

### #358

`$XDG_CACHE_HOME/pinfold/artifacts/init` a symlink to `initdir/` inside a
fresh non-Git project, share first run of `-- true`, interactive.

- before: prompts, saves `pinfold.toml` and the project's `state.json`,
  then fails `pinfold: owner-unavailable`, exit 125. Two new locks.
  `initdir/` stays empty. A configured rerun fails the same way. Removing
  the symlink, the same configured project runs `true`, exit 0, so the
  link was the cause. `host-path-in-project` was not observed: the
  client saw only `owner-unavailable`, and that `mounts::prepare`'s
  check is the refusal behind it comes from code reading. The issue's
  text names `host-path-in-project`.
- after, fresh project and roots: `host-path-in-project: …/initdir/<sha>/linux-arm64
  resolves inside …` before any prompt, exit 125; redirected the same.
  No new lock; the config and state roots are absent; `initdir/` empty.
- after, the configured project from the before run with the link
  restored: refused the same way; `state.json`'s mtime is unchanged.

### Positive controls (after)

- The #357 proxy project with the default external roots: share first
  run, exit 0.
- The #358 project with the link removed: share first run, exit 0.
- A plain project with external roots: share first run (before), then a
  configured share run (after), exit 0.
- A Git project: disk first run, exit 0; configured run, exit 0;
  `git remote add box pinfold::ROOT` and `git push box main` lands
  `main`, exit 0.

### Checks

`cargo fmt --check`; `cargo clippy --all-targets --locked -- -D warnings`;
the same for `--target x86_64-unknown-linux-musl` with zig as the musl C
compiler. A planted unused function failed both clippy runs, so both
check the crate.

## Not changed

- `pinfold stop` takes the project maintenance lock with no protected-path
  check; in a folder holding the lock directory it creates one lock file.
  `stop` is not a checking command in the spec.
- The owner's startup refusal reaches the client as `owner-unavailable`;
  its reason is lost.
- The early check passes no profile. A profile's directory, files or
  share symlinked into the project still refuse under the locks, after
  the two lock files exist; those live outside the project, so nothing
  is created inside it.
- Each protected-path check on macOS now hashes the embedded init in
  `artifacts::init_path`. The cost is unmeasured.
