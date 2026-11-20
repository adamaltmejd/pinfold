# 2026-11-20: setup wording

Commented `config edit` template, shorter first-run prompts, one Rex line,
and the not-set-up intro's `config edit` note. Exercised on macOS (Apple
runtime) with a debug binary. The suite is suspended; these records are
the evidence.

## Setup

`$T` is a fresh temp directory. `$T/pf` runs the debug binary with
`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` under `$T`,
`PINFOLD_PROFILE=default`, `VISUAL=true EDITOR=true` and
`PINFOLD_NO_UPDATE_CHECK=1`. `$T/pf2` is the same without
`PINFOLD_PROFILE`. Projects: `$T/repo`, `$T/repo2`, `$T/repo3` (Git, one
commit) and `$T/plain`, `$T/plain2` (no Git). Prompts were driven by
`expect` with `REX_SESSION` unset. The existing
`profile-default` image was used; nothing was built or removed.

## Bare `pinfold`, not set up (`$T/plain`)

```
Project   $T/plain
Box       not set up

The first run asks you to confirm, and lets you pick a disk instead:
Profile   default
Files     Shared: the box edits $T/plain

Set up and run     pinfold -- CMD     (e.g. pinfold -- claude)
Change settings    pinfold config edit   (creates the settings file)
All commands       pinfold --help
```

A configured, running disk project (`$T/repo3`) still prints
`Change settings  pinfold config edit` without the note.

## `config edit`

In `$T/repo` and `$T/plain`, exit 0 each:

```
Created $T/cfg/pinfold/projects/repo-09c6dafe9f75/pinfold.toml with the default settings (shared folder).
```

A second run prints `Opening PATH`, exit 0. The file is the same in the
Git and the non-Git folder:

```toml
mounts = [{ host = ".", kind = "share" }]
# kind: "share" lets the box edit this folder directly; "disk" gives it its own isolated disk.

# Optional settings: uncomment a line to set it.
# Profile to use (default, documents, adam, or a saved profile):
# profile = "default"
# Local image to run instead of the profile's (not with a Containerfile beside this file):
# image = "localhost/pinfold/image-my-image:latest"
# Hosts the box may reach; replaces the default list:
# allow = ["api.anthropic.com", "github.com"]
# Host services the box reaches through the proxy, by name:
# routes = { db = "127.0.0.1:5432" }
# CPUs and memory for the box:
# cpus = 4
# memory = "8G"
# Variables in the box: a value, or { from = "VAR" } to pass the host's VAR:
# env = { LANG = "C.UTF-8", API_KEY = { from = "MY_API_KEY" } }
# A Containerfile beside this file builds the project's own image.
```

On Linux the comment under the mount reads
`# kind: "disk" gives the box its own isolated disk.` and the mount is
`disk` (not run here; the code path is the `cfg!` branch).

`pinfold config` accepts it (exit 0, profile default, image
`localhost/pinfold/profile-default:latest (available locally)`).
`config --json` for the template and for a file holding only the mount
line are byte-identical (`diff` empty).

Each example line uncommented alone parses with `config --json`, exit 0:
`profile` (default), `image` (`image_source` `explicit`), `allow` (the
two hosts), `routes` (`{"db": "127.0.0.1:5432"}`), `cpus` (4.0),
`memory` (`8G`), `env` (`LANG` literal, `API_KEY` as
`{"from": "MY_API_KEY"}`). All seven together parse, exit 0. Changing
`kind = "share"` to `"disk"` reports `disk`. Values that need real ones
before a run: `image` (the placeholder is not a local image), `routes`
(needs a service at the address), `env`'s `from` (the host variable must
be set at start). Uncommenting `image` beside a project Containerfile
refuses as `image-source-conflict`, which its comment says.

Row 18's sentence holds: `config edit` with `EDITOR=true` wrote the file
and `config` then proceeded.

## First run, share, `pinfold -- true` in `$T/repo2`

```
Profile: default
Files for the box in $T/repo2:
  share  the box edits this folder directly, including .git
  disk   the box gets its own Git clone on an isolated disk (committed files only)
Choose [share/disk] (share)
share
Save these settings and run true? [Y/n]
y
pinfold: git-optional-locks: host `git status` can refresh the index when optional locks are enabled, which can make box commits record files as deleted; set GIT_OPTIONAL_LOCKS=0.
```

Exit 0. Saved file: `mounts = [{ host = ".", kind = "share" }]` and
`profile = "default"`. The `git-optional-locks` line is the existing
share notice.

## First run, disk, `pinfold start` in `$T/repo3`

```
Profile: default
Files for the box in $T/repo3:
  share  the box edits this folder directly, including .git
  disk   the box gets its own Git clone on an isolated disk (committed files only)
Choose [share/disk] (share)
disk
Save these settings and start the box? [Y/n]
y
```

Exit 0; the box ran. Saved file: `kind = "disk"`, `profile = "default"`.
Bare `pinfold` there reported `Box files Isolated disk: …`.
`pinfold stop` exit 0.

## First run, non-Git, profile asked (`$T/plain2`, `PINFOLD_PROFILE` unset)

```
Choose a profile (default, documents, adam, or a saved profile) [default]

Files for the box in $T/plain2:
  share  the box edits this folder directly
  disk   the box starts with an empty isolated disk
Choose [share/disk] (share)
share
Save these settings and run true? [Y/n]
n
```

Exit 125; no configuration file was created. No profile line follows an
asked profile.

## Rex

Not exercised. Bundled profiles do not ship `rex`, so `start` prints no
Rex line, and the user's own Rex app state was not used. The new single
line compiles in `report`:
`pinfold start: rex: add unix://PATH in the Rex app under Add Remote Host… (once); it is reachable while this box runs.`

## Cleanup

`pinfold disk rm` by name: `pi-repo2-aa540fd05ec0-home`,
`pi-repo3-82aafb2e8ac9-home`, `pi-repo3-82aafb2e8ac9-project`. Afterwards
`container ls -a` and `container volume ls` held only what existed
before. `$T` was removed.

## Checks

`cargo fmt --check` and
`cargo clippy --all-targets --locked -- -D warnings` pass.

## Review fixes

Review of PR #360 found four inaccurate texts and asked for one addition.
Rerun with a fresh `$T` set up as above; `$T/pf` takes the profile from
`PF_PROFILE`, default `default`.

- The disk option promised a Git clone or an empty disk. Setup can reuse
  a retained disk, and a new Git disk receives only the host's committed
  branches (First start). New option lines and Linux statement below.
- The `routes` example used a PostgreSQL address; named routes forward
  plain HTTP only. It is now `routes = { api = "127.0.0.1:8080" }` under
  `# HTTP services on this machine the box may reach as http://NAME:`.
  Uncommented, `config --json` reports `{"api": "127.0.0.1:8080"}`.
- The Rex line claimed the relay stays reachable while the box runs; it
  stops if the guest Rex server exits. Now:
  `pinfold start: rex: add unix://PATH in the Rex app under Add Remote Host… (once). It works only while this box runs.`
  Not exercised (no `rex` in bundled images).
- A failed preview after `Box       not set up` dropped the
  `config edit` note. It is carried to the footer.
- Addition: first-run setup saves the same commented template, with the
  chosen mount and profile as active lines.

First run in a Git folder, `pinfold -- true`, share, exit 0:

```
Profile: default
Files for the box in $T/repo:
  share  the box edits this folder directly, including .git
  disk   the box works on an isolated disk; a new disk starts with your committed branches
Choose [share/disk] (share)
share
Save these settings and run true? [Y/n]
y
```

In a non-Git folder, answering `disk` then `n`, exit 125, no file:

```
  share  the box edits this folder directly
  disk   the box works on an isolated disk; a new disk starts empty
```

Linux statement (code only):
`Files for the box in PATH: an isolated disk; a new disk starts with your committed branches.`

The saved first-run file:

```toml
mounts = [{ host = ".", kind = "share" }]
# kind: "share" lets the box edit this folder directly; "disk" gives it its own isolated disk.
# Profile to use (default, documents, adam, or a saved profile):
profile = "default"

# Optional settings: uncomment a line to set it.
# Local image to run instead of the profile's (not with a Containerfile beside this file):
# image = "localhost/pinfold/image-my-image:latest"
# Hosts the box may reach; replaces the default list:
# allow = ["api.anthropic.com", "github.com"]
# HTTP services on this machine the box may reach as http://NAME:
# routes = { api = "127.0.0.1:8080" }
# CPUs and memory for the box:
# cpus = 4
# memory = "8G"
# Variables in the box: a value, or { from = "VAR" } to pass the host's VAR:
# env = { LANG = "C.UTF-8", API_KEY = { from = "MY_API_KEY" } }
# A Containerfile beside this file builds the project's own image.
```

`pinfold config` accepts it, exit 0. `config --json` for it and for the
previous first-run content (`mounts` and `profile = "default"` only) are
byte-identical.

The `config edit` template is the same without the active profile line;
its optional section starts with the profile example:

```toml
mounts = [{ host = ".", kind = "share" }]
# kind: "share" lets the box edit this folder directly; "disk" gives it its own isolated disk.

# Optional settings: uncomment a line to set it.
# Profile to use (default, documents, adam, or a saved profile):
# profile = "default"
# Local image to run instead of the profile's (not with a Containerfile beside this file):
...
```

The rest is as in the saved file. `config --json` accepts it, exit 0;
all seven example lines uncommented together parse, exit 0.

Bare `pinfold` in an unconfigured non-Git folder with
`PINFOLD_PROFILE=nonexistent`, exit 0:

```
Project   $T/plain2
Box       not set up

The first run asks you to confirm, and lets you pick a disk instead:
Project details unavailable. profile "nonexistent" has no directory $T/cfg/pinfold/profiles/nonexistent

Run a command    pinfold -- CMD
Change settings  pinfold config edit   (creates the settings file)
All commands     pinfold --help
```

Cleanup: `pinfold disk rm pi-repo-25bd61ecb3e6-home`; `$T` removed.
`cargo fmt --check` and clippy pass.
