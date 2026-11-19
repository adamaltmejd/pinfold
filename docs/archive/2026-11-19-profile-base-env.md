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
- **Launch record** keeps naming env variables only, literals included,
  as for every box variable. The issue says literals show there; that
  would change the core run record for every caller, so it is left for
  the operator.
- **Join rule.** The snapshot already compares `from` values (hex, in
  memory, over the private socket), so a joining client with another value
  refuses as `config-changed` naming `env.values`, exactly as for
  `PINFOLD_ENV_` and route variables.
- **Missing `from` variable** gives the same `owner-unavailable` (125) as
  a missing injecting-route variable. The diagnostic is poor for both;
  that is a separate ticket.

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
  route variables already are.
- `grep -rl` of the value and of its hex encoding over the config, state
  and cache roots and the lock directory found nothing (exit 1); the
  variable name was found in the profile's `pinfold.toml` (control).
- A second client with `PF343_DUMMY_TOKEN=other-dummy…`: `pinfold:
  config-changed: … Differing settings: env.values.`, exit 125. With the
  same value it joined and printed the token.

### Checks

`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`
and `cargo clippy -p pinfold --all-targets --locked --target
x86_64-unknown-linux-musl -- -D warnings` (zig as ring's C compiler) were
clean at both code commits.

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
