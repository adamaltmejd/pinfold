# 2026-11-20: setup summary and Rex address line

First-run setup states each setting the environment decides, asks the
rest, then prints the settings file's path and its active lines before
the confirm. The Rex line splits so the `unix://` address stands alone.
Exercised on macOS (Apple runtime) with a debug binary. The suite is
suspended; these records are the evidence.

## Setup

`$T` is a fresh temp directory. `$T/pf` unsets every `PINFOLD_*` and
`REX_*` variable, sets `XDG_CONFIG_HOME`, `XDG_STATE_HOME`,
`XDG_CACHE_HOME` and `XDG_DATA_HOME` under `$T`, `VISUAL=true
EDITOR=true`, exports `PINFOLD_PROFILE` only when the run sets
`PF_PROFILE`, and runs the debug binary. Projects `$T/p-start`,
`$T/p-run`, `$T/p-decline`, `$T/p-bad` are plain folders (no Git).
Prompts were driven by `expect`; before answering the confirm, the script
checked that the printed path lies under `$T/config/`. The existing
`profile-default` image was used; nothing was built or removed.

## `pinfold start`, `PINFOLD_PROFILE=default`, choosing disk (`$T/p-start`)

```
Profile  default  (from PINFOLD_PROFILE)
Files for the box in $T/p-start:
  share  the box edits this folder directly
  disk   the box works on an isolated disk; a new disk starts empty
Choose [share/disk] (share)
disk
These settings will be saved to $T/config/pinfold/projects/p-start-62ddf13fae43/pinfold.toml:
  profile = "default"
  mounts = [{ host = ".", kind = "disk" }]
Save and start the box? [Y/n]
y
```

Exit 0; the box ran with an isolated disk. The saved file's lines that
are neither blank nor comments (`grep -v '^#' | grep -v '^$'`):

```
profile = "default"
mounts = [{ host = ".", kind = "disk" }]
```

They equal the summary. The file opens with the profile's comment and
line, then the mount line and its `# kind:` comment, then the commented
optional keys.

## Configured project starts without a dialog (`$T/p-start`)

After `pinfold stop` (exit 0), `pinfold start` under `expect`, which
failed on any `[Y/n]`, `[default]` or `(share)` prompt: no output, exit
0, a new box running. Stopped again (exit 0).

## `pinfold -- true`, `PINFOLD_PROFILE` unset, Enter twice (`$T/p-run`)

```
Choose a profile (default, documents, adam, or a saved profile) [default]

Files for the box in $T/p-run:
  share  the box edits this folder directly
  disk   the box works on an isolated disk; a new disk starts empty
Choose [share/disk] (share)

These settings will be saved to $T/config/pinfold/projects/p-run-ffe6bd32b088/pinfold.toml:
  profile = "default"
  mounts = [{ host = ".", kind = "share" }]
Save and run true? [Y/n]
y
pinfold: git-optional-locks: host `git status` can refresh the index when optional locks are enabled, which can make box commits record files as deleted; set GIT_OPTIONAL_LOCKS=0.
```

Exit 0. The file's active lines equal the summary. The box ended with
the command.

## Declining saves nothing (`$T/p-decline`)

`PINFOLD_PROFILE=default`, `share`, then `n` at `Save and start the box?
[Y/n]`: exit 125, no `p-decline-*` directory under
`$T/config/pinfold/projects/`.

## A bad mount answer refuses (`$T/p-bad`)

```
Choose a profile (default, documents, adam, or a saved profile) [default]

Files for the box in $T/p-bad:
  share  the box edits this folder directly
  disk   the box works on an isolated disk; a new disk starts empty
Choose [share/disk] (share)
both
pinfold start: unknown files choice "both"; answer share or disk
```

Exit 125, no summary, no file.

## `config edit` template unchanged

`pinfold config edit` in `$T/p-decline` (exit 0) created a file that
starts `mounts = [{ host = ".", kind = "share" }]`, then the `# kind:`
line, a blank line and the optional keys, as before. Only setup's file
moved the profile above the mount.

## Not run

- The Linux stated mount line (`Files    disk  (the only kind on Linux;
  a new disk starts empty)`) was compiled by the Linux-target clippy,
  not run.
- The Rex lines were compiled, not run: bundled images have no `rex`,
  and the relay needs the user's Rex app. `rex::report` sends each line
  through `forward_diagnostics` separately, which escapes control
  characters and adds no prefix, so the second line is exactly
  `unix://PATH`.

## Cleanup

Disks `pi-p-run-ffe6bd32b088-home`, `pi-p-start-62ddf13fae43-home` and
`pi-p-start-62ddf13fae43-project` removed with `pinfold disk rm`; no box
of these projects remained; `$T` deleted.
