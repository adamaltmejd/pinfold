# Project configuration build (#294)

Date: 2026-11-14, on top of main 759574b. This is step 2 of
`2026-11-12-project-box-plan.md`. The file records what the build
session ran on its Mac with the real binary, and what it did not run.
This version follows the operator's review of PR #299.

## How it was run

- Binary: `cargo build` (debug) of this branch, on macOS arm64.
- Script: `2026-11-14-project-config-build/no-box-cases.sh BINARY`.
  - It makes a fresh directory under `$TMPDIR` and sets `XDG_CONFIG_HOME`
    and `XDG_STATE_HOME` inside it.
  - It writes a fresh maintenance stamp into the state root first, so the
    daily pass does not run.
  - It unsets pinfold's environment overrides and sets
    `PINFOLD_NO_UPDATE_CHECK=1`.
  - It creates a Git project (`proj`, one empty commit) and a plain folder
    (`plain`).
  - Every case in it is refused before a box, or only reads.
- Full output, with paths rewritten (`/Users/user`, `$TMPDIR`):
  `2026-11-14-project-config-build/no-box-output.txt`.
- The session's sandbox denies Apple `container` ("Operation not
  permitted"), so no box started. The cases that start one are in
  `run-box-cases.sh` (below), not run.
- Linux was not run. The Linux `not-git` refusal is compiled only for
  Linux; clippy for `x86_64-unknown-linux-musl` was clean, nothing more.

## Observed

1. **No configuration.** In `proj` with no file, `config`, `pi`, `build`,
   `push` and `fetch` each exit 1 with
   `no-config: …/proj has no configuration; run `pinfold config edit``.
   Afterwards the config root does not exist and no project state
   directory exists. `doctor` exits 0, prints its host report and a
   `config: no-config: …` line.
2. **`.pinfold.toml` is not read.** With `allow = ["example.com"]` in
   `proj/.pinfold.toml` and no host file, `config` exits 1 with `no-config`.
3. **`pinfold allow` is gone.** It prints the usage and exits 1. The usage
   has no `allow` line and has a `config edit` line.
4. **`config edit`.** `config edit --help` prints `pinfold config edit`,
   exit 0. `EDITOR=true pinfold config edit` exits 0. It writes
   `$XDG_CONFIG_HOME/pinfold/projects/proj-<hash>/pinfold.toml`, holding
   exactly `mounts = [{ host = ".", kind = "disk" }]`, and nothing else in
   that directory. `config` then exits 0. Its report has `config` (the
   file) and no `mode`, `config_source`, `trust` or `mounts`. After
   `allow = ["only.example"]` is appended, a second `config edit` leaves
   the file byte-identical (cmp). `EDITOR=false` exits 1.
   `VISUAL="head -n1" EDITOR=false` runs VISUAL and exits 0.
5. **Mount validation.** Each of these exits 1 with
   `spec: `mounts` must be exactly mounts = [{ host = ".", kind = "disk" }]`:
   - `kind = "share"`
   - `host = "/tmp"`
   - `mounts = "x"`
   - `host = 1`
   - a missing `kind`
   - an extra `guest` key
   - two entries
   - no `mounts`

   The `[[mounts]]` table form of the project entry is accepted (exit 0).
   The old top-level `containerfile` key is refused, naming it.
6. **A profile's `mounts`.** A user profile `withmounts` whose
   `pinfold.toml` holds the project mount, selected with
   `PINFOLD_PROFILE`, exits 1 with
   `profile "withmounts" pinfold.toml: `mounts` is a project key`.
7. **Project recipe.** A `Containerfile` beside the file makes `config`
   report `image_source` `project`. Adding `image = …` exits 1 with
   `image-source-conflict`. With `image = …` and no Containerfile, `build`
   exits 1 with `image-no-recipe`.
8. **Dangling symlink.** The file is a symlink to a missing path.
   - Target inside the project: `config` exits 1 with
     `file is not readable`. `config edit` exits 1 with
     `host-path-in-project` and does not create the target.
   - Target outside the project: `config edit` exits 1 with
     `file is not readable` and does not create the target.
9. **No global file.** `$XDG_CONFIG_HOME/pinfold/pinfold.toml` holds
   `allow = ["global.example"]`, and the project file has no `allow`.
   `config` reports the built-in default list.
10. **Moved project.** A copy of `proj` at `proj-moved` exits 1 with
    `no-config`.
11. **`host-path-in-project`.** Each refusal below exits 1 with
    `host-path-in-project: <path> resolves inside …/proj`.
    - **Cache root through a symlink:** `XDG_CACHE_HOME` is
      `cachelink/missing/cache`. `cachelink` is a symlink to
      `proj/link-target`, and `missing` does not exist. `pi`, `build` and
      `push` are refused, naming `…/cache/pinfold/artifacts`. Nothing is
      created under `link-target`.
    - **Config root inside the checkout:** `XDG_CONFIG_HOME` is
      `proj/cfg`. `config edit` is refused, and `proj/cfg` is not created.
    - **The project's file symlinked into the checkout:** under a third
      config root, `pinfold.toml` links to `proj/policy.toml`. `pi`,
      `build` and `push` are refused, naming it.
    - **The project's `Containerfile` symlinked into the checkout:**
      `pi` and `build` are refused, naming it.
    - **A user profile's `pinfold.toml` symlinked into the checkout:**
      profile `p` has a plain `Containerfile`, and its `pinfold.toml`
      links to `proj/profile.toml`. With `PINFOLD_PROFILE=p`, `pi` and
      `build` are refused, naming it.
12. **A folder name with a newline.** `config edit` in a Git project named
    `new\nline` exits 0. The file holds only the project mount.
13. **macOS outside Git.** In `plain`, `config` exits 1 with
    `no-config`. `EDITOR=true pinfold config edit` exits 0, and `config`
    then exits 0.

Checks:
- `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`
  are clean on macOS.
- `cargo clippy --target x86_64-unknown-linux-musl --all-targets --locked -- -D warnings`
  is clean. It was run with `RUSTC_WRAPPER` unset and with
  `CC_x86_64_unknown_linux_musl` set to a wrapper. The wrapper drops
  cc-rs's `--target=` and runs `zig cc -target x86_64-linux-musl`.

## Earlier run, corrected

The first version of this record (commit b0a27ee) also ran some controls
that passed the configuration gates. It said `pi` "stopped at the lock
directory" and that a scratch `pi` stopped at the image because of the
sandbox. Both were inferred: the outputs were
`Operation not permitted (os error 1)` and
`image localhost/pinfold/profile-default:latest is missing`, and neither
names a cause. Those controls now live in `run-box-cases.sh`.

That run also reported `image_built: false` with `image_error: null`
while `container` was denied. `service::inspect` treats any nonzero exit
of `container image inspect` as "not resolved", so a denied or stopped
service reads as "not built". Guarantee 18 expects `image_built: null`
and an `image_error` when the probe is unavailable. That code is
unchanged from main; it is reported to the operator, not fixed here.

## Not run

All of these are "not run (sandbox)".
`2026-11-14-project-config-build/run-box-cases.sh BINARY` runs them
outside the sandbox; its output is added here before merge.

The script:
- was only syntax-checked: `container system status` and
  `container image inspect`, which it calls, were never run;
- writes a fresh maintenance stamp into its state root;
- uses the real cache root (pinned artifacts and the init), except for
  case C's refusal.

Cases:
- A. A project with no configuration creates no box.
- B. A configured Git project starts (`pinfold pi --version`), and `push`
  and `fetch` work against it.
- C. A cache root inside the project, through a symlink with a
  nonexistent suffix, is refused with no box. The same project with
  external roots starts.
- D. On macOS outside Git, scratch is refused without a configuration and
  starts with one.

Also not run:
- Linux: every project command outside Git refused as `not-git`. There
  was no Linux host in this session.
- The assertions that move from guarantee 12 to 13 (the identity
  `pi <pi@pinfold.invalid>`, and the start's push and fast-forward). That
  code is unchanged and was not exercised.
