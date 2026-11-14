# Project configuration build (#294)

Date: 2026-11-14, on top of main 759574b. This is step 2 of
`2026-11-12-project-box-plan.md`. The file records what the build session
ran on its Mac with the real binary, and what it did not run.

## How it was run

- Binary: a `cargo build` (debug) of this branch, on macOS arm64.
- Script: `2026-11-14-project-config-build/no-box-cases.sh BINARY`. It
  makes a fresh directory under `$TMPDIR`. It sets `XDG_CONFIG_HOME` and
  `XDG_STATE_HOME` inside it, and sets `PINFOLD_NO_UPDATE_CHECK=1`. It
  creates a Git project (`proj`, one empty commit) and a plain folder
  (`plain`).
- Full output, with paths rewritten to `/Users/user` and `$TMPDIR`:
  `2026-11-14-project-config-build/no-box-output.txt`.
- The session's sandbox denies Apple `container` ("Operation not
  permitted"). It also denies writes to the lock directory
  `/private/tmp/pinfold-501`. So no box started. Every `pi` or `build`
  below that passes the configuration gates stops at one of those two
  denials. It is recorded as "passed the gate, stopped at X", not as a
  started box.
- Linux was not run. The Linux-only refusal (`not-git` outside Git) is
  compiled only for Linux. Clippy for `x86_64-unknown-linux-musl` was
  clean; nothing more was run for Linux.

## Observed

1. **No configuration.** In `proj` with no file, `config`, `pi`, `build`,
   `push`, `fetch` and `doctor` each exit 1 with
   `no-config: …/proj has no configuration; run `pinfold config edit``.
   Afterwards the config root does not exist. The state root holds only
   `pinfold/maintenance`, the daily pass's stamp; that pass runs before
   the verb, as it did before this change. There is no project state.
2. **`.pinfold.toml` is not read.** With `allow = ["example.com"]` in
   `proj/.pinfold.toml` and no host file, `config` still exits 1 with
   `no-config`.
3. **`pinfold allow` is gone.** It prints the usage and exits 1. The usage
   has no `allow` line and has a `config edit` line.
4. **`config edit`.** `config edit --help` prints `pinfold config edit`
   and exits 0. `EDITOR=true pinfold config edit` exits 0 and writes
   `$XDG_CONFIG_HOME/pinfold/projects/proj-<hash>/pinfold.toml`:

   ```toml
   # pinfold configuration for $TMPDIR/pf.XXXX/proj
   mounts = [{ host = ".", kind = "disk" }]
   ```

   `config` then exits 0. Its report has:
   - `config`: the file's path;
   - `mounts`: `[{"host":".","kind":"disk"}]`;
   - `containerfile`: `"profile"`;
   - the home and project disks;
   - no `mode`, `config_source` or `trust`.

   `image_built` was false. Here the sandbox's denied image listing reads
   as absent; this was not checked further.

   After `allow = ["only.example"]` was appended, a second
   `EDITOR=true pinfold config edit` left the file byte-identical (cmp).
   `EDITOR=false` exits 1. `VISUAL="head -n1" EDITOR=false` runs VISUAL,
   which prints the comment line, and exits 0.
5. **Mount validation.** Each of these exits 1 with
   `spec: `mounts` must be exactly mounts = [{ host = ".", kind = "disk" }]`:
   `kind = "share"`, `host = "/tmp"`, no `mounts`, and two project
   entries. The `[[mounts]]` table form of the same entry is accepted
   (exit 0). An extra key `guest` is refused, naming it. So is the old
   top-level `containerfile` key.
6. **A profile's `mounts`.** A user profile `withmounts` has the project
   mount in its `pinfold.toml`. Selected with `PINFOLD_PROFILE`, it exits 1
   with `profile "withmounts" pinfold.toml: `mounts` is a project key`.
7. **Project recipe.** With a `Containerfile` beside the file, `config`
   reports:
   - `image_source`: `project`;
   - `containerfile`: its absolute path;
   - `image`: `localhost/pinfold/project-proj-<hash>:latest`.

   Adding `image = …` to the file exits 1 with `image-source-conflict`.
8. **Dangling symlink.** With the file replaced by a symlink to a missing
   path, `config` exits 1 with `…/pinfold.toml: file is not readable`.
9. **No global file.** `$XDG_CONFIG_HOME/pinfold/pinfold.toml` holds
   `allow = ["global.example"]`, and the project file has no `allow`.
   `config` reports the built-in default list.
10. **Moved project.** A copy of `proj` at `proj-moved` exits 1 with
    `no-config`.
11. **`host-path-in-project`.** The setup:
    - `XDG_STATE_HOME` is `statelink/missing/state`;
    - `statelink` is a symlink to `proj/link-target`;
    - `missing` does not exist;
    - the project's file is copied to a second config root.

    `pi`, `build` and `push` each exit 1 with
    `host-path-in-project: …/statelink/missing/state/pinfold resolves inside …/proj`.
    The daily maintenance pass had already made `pinfold/maintenance`
    under that state root, inside the project. This order predates the
    change; see Open.

    Control, the same project with external roots: `pi` passed the gate
    and stopped at the lock directory
    (`Operation not permitted (os error 1)`). `build` passed it and
    stopped at the container build.
12. **The project's own files symlinked into the project.** Under a third
    config root, the project's `pinfold.toml` is a symlink to
    `proj/policy.toml`. `pi`, `build` and `push` each exit 1 with
    `host-path-in-project: …/projects/proj-<hash>/pinfold.toml resolves inside …/proj`.
    With a plain file there and the `Containerfile` a symlink to
    `proj/Recipe`, `pi` and `build` exit 1 naming the `Containerfile`.
    The first version of this change missed this case; review found it
    before the PR.
13. **macOS outside Git.** In `plain`:
    - `config` exits 1 with `no-config`;
    - `EDITOR=true pinfold config edit` exits 0;
    - `config` then exits 0, with `project.disks` null;
    - `pi` passed the configuration gates and stopped at
      `image localhost/pinfold/profile-default:latest is missing`, which
      is the sandbox's denied image listing.

Checks:
- `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`
  are clean on macOS.
- `cargo clippy --target x86_64-unknown-linux-musl --all-targets --locked -- -D warnings`
  is clean. It ran with `RUSTC_WRAPPER` unset and with
  `CC_x86_64_unknown_linux_musl` set to a wrapper. The wrapper drops
  cc-rs's `--target=` and runs `zig cc -target x86_64-linux-musl`.

## Not run

All of these are "not run (sandbox)".
`2026-11-14-project-config-build/run-box-cases.sh BINARY` runs them
outside the sandbox; its output is added here before merge. The script
was only syntax-checked: `container system status` and
`container image inspect`, which it calls, were never run. It writes a
fresh maintenance stamp into each state root it uses, so the daily pass
does not prune boxes or images host-wide.

- A. A project with no configuration creates no box.
- B. A configured Git project starts (`pinfold pi --version`), and `push`
  and `fetch` work against it.
- C. `host-path-in-project` creates no box, and the same project with
  external roots starts.
- D. On macOS outside Git, scratch starts once the folder has a
  configuration.

Also not run:
- Linux: `pinfold pi` outside Git refused as `not-git`. There was no Linux
  host in this session.
- The assertions that move from guarantee 12 to 13 (the identity
  `pi <pi@pinfold.invalid>`, and the start's push and fast-forward). That
  code is unchanged and was not exercised.

## Open

- The daily maintenance pass runs before any verb and writes its stamp
  under the state root. So a state root inside the project gets a
  directory before `host-path-in-project` refuses. This predates the
  change, and the spec now says "before any project state". Whether the
  pass should skip on that refusal is a question for the operator.
