# Profile base and configured env

2026-11-19. Issues #343 (a profile builds on another profile; `full`
renamed `adam`) and #344 (configuration sets box environment variables).
This records what was built, the choices the issues left open, and what
the real binary did on macOS and Linux. No test was added: the suite is
suspended.

## Built

- `base = "NAME"` in a profile's `pinfold.toml`. `Profile::load` resolves
  the chain; a cycle, a self-reference or a missing base refuses as
  `profile`. Configuration folds each link's file root first, then the
  project and environment. Seeds and share files lie over the base's by
  path.
- Builds run over the chain's recipe links root first, then the project.
  Each image records the id of the one built just beneath it in
  `dev.pinfold.base`. Linux's existing freeze of that label, which only
  accepted project builds, now accepts any managed build (profile or
  project label set). No second mechanism was added.
- Freshness walks the chain from the root: an image is `image-outdated`
  when its recipe changed, its recorded base is not the base's current id,
  or its base is outdated.
- Protected paths cover every link's directory and files.
- Built-ins: `profile/image/default.Containerfile` (the former base,
  agents and harden fragments), `documents.Containerfile` (`FROM
  profile-default`, the former bun and documents fragments) and
  `adam.Containerfile` (`FROM profile-documents`, the former full
  fragment). Each ends with the setuid strip. Only `default` resolves and
  installs agents. `documents` and `adam` carry `base` in their
  `pinfold.toml`.
- `env` table: `plan::Env` reused as the configuration value, merged name
  by name; `PINFOLD_ENV_<NAME>` is applied last as `{ from =
  "PINFOLD_ENV_<NAME>" }`. `config --json` serializes declarations.
- Deleted: the four fragment files and their concatenation, the
  `Share::Embedded` static-slice form (now `Share::Files`, owned), the spec
  line "A built-in never builds from another profile's image" and the
  project-only filter on Linux's base freeze.

## Choices the issues left open

- **Share overlay.** A chain with one share mounts it as before (a user
  directory stays live). Two or more shares mount as a content-addressed
  copy of the merged files, bytes and executable bit, made at start like
  the built-ins' embedded files. Symlinks in a user share are not copied.
- **`FROM` is the user's.** A profile recipe names its nearest recipe
  base's stable ref; pinfold does not parse or enforce it, as for project
  recipes.
- **Recipe-less links.** A profile without a Containerfile uses the
  nearest base image; `build --profile` on it builds the chain and prints
  that image's ref.
- **Freshness command.** Every chain warning names the selected top's
  build command, since building any link rebuilds the chain beneath it.
- **`profile new --from X`** copies X's own files and its `base`, not the
  flattened chain.
- **`base` in a project file** is refused, as `mounts` is in a profile.
- **`env` merges name by name** (the other keys replace whole). An
  inherited name cannot be unset.
- **Launch record** names env variables only, literals included, as for
  every box variable. #344 says literals show there; the operator
  accepted names only, since values would change the core run record for
  every caller.
- **Join rule.** The snapshot already compares `from` values (hex, in
  memory, over the private socket), so a joining client with another value
  refuses as `config-changed` naming `env.values`, exactly as for
  `PINFOLD_ENV_` and route variables. #344 says the value never appears in
  the snapshot; the operator accepted it there, on condition that it
  reaches no file. Review on PR #350 found that it does on Apple (see
  Review fixes).
- **Missing variable.** First built as the owner's refusal, which reached
  the client only as `owner-unavailable` (125), for env and route
  variables alike. Fixed in a follow-up: project admission
  (`validate_project`) checks that every env `from`, injecting header and
  login route variable is set, by presence only, and the client prints the
  variable's name.

## Setup

Both platforms used temporary projects and private XDG roots,
`PINFOLD_NO_UPDATE_CHECK=1`, `GIT_CONFIG_GLOBAL=/dev/null`,
`GIT_CONFIG_NOSYSTEM=1` and `PINFOLD_PROFILE` unset. The dummy secret was
`dummy-secret-7f3a9c` in `PF343_DUMMY_TOKEN`.

- **macOS:** 27.2, Apple `container`, debug build. Apple's image store is
  shared with the user's own images, so every link was a uniquely named
  throwaway copy: `pf343-default` (`profile new --from default`),
  `pf343-documents` and `pf343-adam` (copies with `base` and `FROM`
  pointed at the `pf343-` names), and `pf343-local` (`base =
  "pf343-adam"`, a two-line Containerfile installing `tree`, a replacing
  `home/.codex/AGENTS.md`, a new `home/local/seed.txt`, a replacing
  `share/pi/skills/read-documents/SKILL.md`, an executable
  `share/bin/hello`, `allow = ["api.github.com"]`). Share project.
- **Linux:** the dev VM, Debian 13, kernel 6.12.105, Firecracker, musl
  debug builds of origin/main (48d2f95) and of this branch, each with its
  own empty roots. The real built-in names were built there. Disk
  project, user profile `pf-local` on `adam` with the same additions.

## Observed

### Build time of the adam image, Linux, empty roots

- origin/main, `pinfold build --profile full`: one build, **172 s**.
- This branch, `pinfold build --profile adam`: three builds (default,
  documents, adam), **376 s**. From the run records: default about
  2 min 24 s, documents about 2 min, adam about 1 min 40 s. Each link pays
  its own builder VM, local-base copy and converter VM.
- Unchanged warm rebuild of the chain: 96 s; every `latest` kept its id
  (default `3ae8a324…`, documents `3b5c96c9…` with base `3ae8a324…`, adam
  `e9211c00…` with base `3b5c96c9…`).

### Chain (both platforms)

- `build --profile pf343-local` (macOS, 42 s from the warm shared cache)
  and `build --profile pf-local` (Linux, 219 s including the warm chain)
  printed `building profile` for each link root first and published each.
  macOS labels: documents base = default's id, adam base = documents' id,
  local base = adam's id.
- In the top profile's box: `pi`, `claude`, `codex`, `bun`, `anydoc`,
  `pdftotext`, `gh` and `tree` on PATH; `/opt/pinfold/packages` holds
  `pi-fff` and `ponytail`; `~/.codex/AGENTS.md` is the top's replacement,
  `~/.pi/agent/settings.json` the base seed (and on macOS
  `~/local/seed.txt` the top's own); `/opt/pinfold/profile` holds
  operating-context, adam's `package.json` (it names `pi-fff`), the top's
  `read-documents/SKILL.md`, and on macOS `bin/hello` mode `-rwxr-xr-x`
  printing its line. No setuid/setgid file. macOS `PINFOLD_ALLOW` was the
  top's `api.github.com` alone; Linux `config --json` ended with adam's
  `github.com`, `api.github.com`.
- **Root change.** macOS, appending a comment to `pf343-default`'s
  recipe: the next `pinfold -- true` and `doctor` printed four
  `image-outdated` lines: default "has changed or unrecorded Containerfile
  inputs", documents, adam and local "builds on an outdated image", each
  with `run \`pinfold build --profile pf343-local\``. After the chain
  rebuild: none. Adding a `RUN` to the root and rebuilding only the root:
  documents "was built from image c2b82f8d…; its base is now 28332fe9…",
  adam and local "builds on an outdated image"; after the chain rebuild the
  box read the root's new `/etc/pf343-root-marker`. Linux, with a user
  `default` (copy of the built-in plus a comment) replacing the built-in
  root: the same four lines for default, documents, adam and `pf-local`;
  removing the override silenced them.
- **Refusals.** `cyc-a`↔`cyc-b`, `selfref` and `orphan` (base
  `nonesuch`): `box up` printed `"reason":"profile"` with `profile
  "cyc-a": profile "cyc-b": base cycle back to profile "cyc-a"`, `profile
  "selfref": base cycle back to profile "selfref"` and `profile "orphan":
  profile "nonesuch" has no directory …`; `pinfold config` printed the
  same and exited 1. Both platforms. A project file with `base = "adam"`:
  "`base` is a profile key", exit 1.
- `PINFOLD_PROFILE=full pinfold config`: `profile "full" has no directory
  …`, exit 1 (both).
- **No base unchanged.** macOS `pf343-nobase` (no `base`, a `share/`
  directory, an explicit image): after `pinfold start`, `cat
  /opt/pinfold/profile/live.txt` printed `one`; after a host edit, the
  next command in the same box printed `two`, so it is still a live mount.
- `profile new pf343-copy --from adam` wrote `Containerfile`,
  `pinfold.toml` (first line `base = "documents"`) and
  `share/pi/package.json` only.
- `default`, `documents` and `adam` resolve in `config --json` with their
  own `profile-<name>` image refs; `adam`'s allow list adds `github.com`
  and `api.github.com`.

### env (both platforms)

Profile (adam link on macOS, `pf-local` on Linux): `PF_LITERAL =
"profile-literal"`, `PF_TOKEN = { from = "PF343_DUMMY_TOKEN" }`,
`PF_SHARED = "from-profile"`. Project: `PF_SHARED = "from-project"`
(macOS also `PF_PROJECT = "project-only"`). Caller:
`PINFOLD_ENV_PF_PROJECT=pinfold-env-wins`.

- With `PF343_DUMMY_TOKEN` unset, `pinfold -- true`: `pinfold:
  owner-unavailable`, exit 125. A missing injecting-route header variable
  gave the same line (macOS).
- `config --json` `env`: `{"PF_LITERAL": "profile-literal", "PF_PROJECT":
  {"from": "PINFOLD_ENV_PF_PROJECT"}, "PF_SHARED": "from-project",
  "PF_TOKEN": {"from": "PF343_DUMMY_TOKEN"}}`.
- In the box: `PF_LITERAL=profile-literal`, `PF_TOKEN=dummy-secret-7f3a9c`,
  `PF_SHARED=from-project`, `PF_PROJECT=pinfold-env-wins`.
- Launch record `env`: `["HOME", "PF_LITERAL", "PF_PROJECT", "PF_SHARED",
  "PF_TOKEN", "PINFOLD_ALLOW"]`.
- With the box kept by `pinfold start`: no process argv held the value
  (`ps -axww` / `ps -eww`); on Linux no Firecracker cmdline held it. The
  value is in the environment of the detached project owner (Linux
  `/proc/PID/environ` of `pinfold __project-owner` alone; macOS `ps -E`
  matched three same-user processes, not identified), as `PINFOLD_ENV_` and
  route variables already are. A follow-up run on macOS passed the value
  only in `pinfold start`'s own environment and listed `ps -axwwE` with
  pids. Exactly two processes held it: the detached `pinfold
  __project-owner` (ppid 1) and its child `container run`. The earlier
  third match was the test shell, which had exported the variable. No
  process outside the owner and its children held the value.
- `grep -rl` of the value and of its hex encoding over the config, state
  and cache roots and the lock directory found nothing (exit 1); the
  variable name was found in the profile's `pinfold.toml` (control).
- A second client with `PF343_DUMMY_TOKEN=other-dummy…`: `pinfold:
  config-changed: … Differing settings: env.values.`, exit 125. With the
  same value it joined and printed the token.

- Edge names (macOS, project `env`, explicit local image):
  `"1BAD"` refused with `invalid box spec: env name "1BAD" must match
  [A-Za-z_][A-Za-z0-9_]*`; `HTTPS_PROXY = "http://evil:1"` arrived as the
  always-applied `http://127.0.0.1:3128`; `PINFOLD_ALLOW = "example.com"`
  replaced the variable's text, as the box spec's own env does (policy
  still comes from `allow`); `HOME = "/tmp/elsewhere"` refused with
  `$HOME=/tmp/elsewhere is not on a disk`, as `PINFOLD_ENV_HOME` would.
- **Follow-up, missing variable (macOS, after the fix).** With an env
  `from = "PF343_MISSING_ENV"` unset: `pinfold: environment variable
  PF343_MISSING_ENV is not set`, exit 125. With an injecting route header
  `from = "PF343_MISSING_ROUTE"` unset: the same line naming
  `PF343_MISSING_ROUTE`, exit 125. Control, both set to dummy values: the
  box printed `PF_TOKEN=dummy-a`, exit 0.
- **Follow-up, running box whose base image was rebuilt (macOS).** Chain
  `pf343b-top` (`FROM` `pf343b-root`) built, then `pinfold start`. Root
  recipe changed and only the root rebuilt: a joining `pinfold -- …`
  joined the running box (exit 0) without a warning, and it still lacked
  the root's new file. Admission ignores recipe freshness, and the selected
  image's id had not changed. `doctor` reported the top `image-outdated`
  ("was built from image a6157877…; its base is now ce52a521…"). After
  the top was rebuilt too, a join refused with `config-changed … Differing
  settings: image.id.`, exit 125. After `pinfold stop`, a new start read
  the root's new file. All of this matches the spec, so nothing was
  changed.
- Cleanup: images are only pruned per source after a build, never
  `latest`, so a base no project selects is not removed.

### Review fixes (PR #350)

- **Apple keeps box env in its own state.** macOS, `container` 1.7.1,
  a kept project box with `PF_FROM = { from = "PF343_FROM" }` and
  `PINFOLD_ENV_PF_ENV`, both dummies. `container inspect` held both hex
  values (once each) under `PINFOLD_BOX_ENV_PF_FROM` and
  `PINFOLD_BOX_ENV_PF_ENV`, and every other box variable the same way;
  the plain values did not appear. Of Apple's metadata files, the box's
  `containers/<box>/config.json` and `runtime-configuration.json` held
  them, both `-rw-r--r--` owned by the user, under `com.apple.container`
  (`drwxr-xr-x`). When the owner ended, the box and that directory were
  removed. origin/main passes env through the same `guest_env`, so
  `PINFOLD_ENV_` values were already recorded there. A full-tree grep
  of Apple's 26 GB directory was stopped as too slow; images, snapshots
  and volumes were not searched. Linux sends env over vsock after boot
  (`vsock::set_env`); with a `from` dummy in a running box, `grep -rlI`
  of the value and its hex over the private roots, `/tmp` and
  `/run/user/<uid>` listed nothing. The spec and README now state the
  Apple exception; the fix is ticketed separately.
- **Merged share per start.** The content-addressed cache with its
  manifest is gone. A merged share is written at each start into the
  box's state dir (`profile-share`), which the owner already removes when
  the generation ends and with a dead owner's record. macOS and Linux,
  `pf343s` (`base = "default"`, own `share/`): the copy existed while
  the box ran and none remained after `stop`, in state or cache.
- **File over directory.** `pf343s` with `share/pi` a file (the base has
  `pi/`) and `home/.codex` a file (the base seeds `.codex/…`): on both
  machines the box saw only the upper `pi` file and an executable
  `extra/run.sh`, and `~/.codex` was the upper file. A first Mac run
  showed `.codex` as a directory: it reused a home disk from the
  aborted env run, and seeds are written only when missing. A fresh disk
  gave the file.
- **HOME.** A configured `HOME = "/tmp/not-home"` is ignored: the box
  printed `HOME=/home/pi` on both machines. The always-applied names
  already won in the runtime.
- **Removed base.** macOS `pf343t` (`base = "pf343r"`) built; doctor
  showed no hint. With `base` removed and the Containerfile unchanged,
  doctor and a launch printed `image-outdated: image …pf343t:latest was
  built from image 5e2dee50…; it now has no base`, and the launch
  proceeded (exit 0).
- **Recipe over a recipe-less base.** A user `documents` (`base =
  "default"`, no Containerfile) under built-in `adam`: `pinfold build
  --profile adam` refused, exit 1, `profile "adam" has a Containerfile,
  so its base "documents" needs one too (its image is "default"'s)`.
  This is a refusal, not a build argument: user recipes write a literal
  `FROM` too, and a build argument would need the builder protocol.
- The first-run prompt offers `adam`. README names `config --json` for
  literals and limits agent re-resolution to chains ending in the bundled
  recipe.

### Re-review fixes (PR #350, after 0b440d7)

- **Direct base.** The guard no longer asks whether any ancestor has a
  recipe: a profile with a Containerfile refuses when its direct base has
  none. macOS: a user `default` with an empty `pinfold.toml` and no
  Containerfile, under bundled `documents`: `build --profile documents`
  and `config --json` both refused, exit 1, `profile "documents" has a
  Containerfile, so its base "default" needs one too`. With a one-line
  user `default` Containerfile, `config --json` resolved `documents` to
  `localhost/pinfold/profile-documents:latest` (exit 0). It was not
  built: the real names are the user's images in the shared store.
- **Merged-share path.** The copy's path is now checked as spec mount
  paths are (no `,` or control byte) when pinfold computes it, so the
  client refuses before an owner starts. macOS, `XDG_STATE_HOME` with a
  comma and a merged share: `pinfold: mount path "…/st,ate/pinfold/boxes/
  …/profile-share" must not hold ',' or an ASCII control character`, exit
  125. Control without the comma: the box read the merged file, exit 0.
- **HOME in `config --json`.** With `HOME` and `PF_LIT` configured, `env`
  showed only `{"PF_LIT": "lit"}`.
- Spec: the box spec's env bullet cites the Apple exception (#351), and
  the Configuration text limits it to variables pinfold passes. README
  says plainly that on macOS Apple stores reversible values on disk and
  shows them in `container inspect` while the box runs, and that agent
  re-resolution applies to chains containing the bundled `default`.

### Checks

`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`
and `cargo clippy -p pinfold --all-targets --locked --target
x86_64-unknown-linux-musl -- -D warnings` (zig as ring's C compiler) were
clean at both code commits and at the review fixes.

## Not run

- The real `default`/`documents`/`adam` names were not built on macOS:
  they are the user's images in the shared store. The Mac chain used
  copies under `pf343-` names.
- The empty-cache timing ran only on Linux; the Apple builder cache is
  shared and cannot be emptied.
- Linux `image-base-changed` when an image beneath moves during a build
  was not provoked.
- A user profile replacing a built-in base was only exercised for hints
  (Linux, `default`), not built.
