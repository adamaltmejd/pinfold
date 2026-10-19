# Reproduction bundle

Omitted from this archive copy: every script, result, patch, log, manifest and
receipt, and `matchlock/`. Only the prose files are kept, so the steps below
record what was run; they cannot be replayed from this directory.

2026-10-15. Research only. These scripts create disposable repositories and
exercise real filesystems/runtimes. They are not production support, an importer,
or replacements for Pinfold's guarantee tests. Run on a trusted disposable host
or inspect the commands before using the explicitly named scratch directory.
Do not run competing Apple Container/BuildKit suites.

## Baseline

Primary source: `<checkout>`, branch
`codex/issue-89-bounded-listing`, HEAD
`198600b1751d889d354ed3657d8f839e8436e469` plus `baseline.patch`.
Its SHA-256 is `d553421a48a119d7d6eef39fa85b7aff691441b0dca05800c9ef2624008cc9c7`.
`source-manifest.json` hashes the 285 tracked/nonignored files observed before
experiments. The build copy excludes dated archive/investigation material,
which contributes no compiled source. `verification.json` confirms every
original captured file and the full tracked diff remained unchanged.
The baseline build's SHA-256 is
`f87a01813fdbbdca12bd418b047b53b275ca99f3835e8f1d4177b92f8560bf26`.
It was not installed. `build.log` records the build. No production patch was
made; the retained patch is the existing dirty baseline, not proposed work.

The executable's dependency/toolchain versions are pinned by that source.
The experiment used macOS 27.0.1 (26A434), arm64, Apple Container 1.5.0,
Git 2.56.0, Cargo/Rust 1.99.0. Installed Pinfold reported 0.2.4 but was not
used as an inferred match for this dirty source.

## Mac fixture setup and order

Scripts use the explicit scratch path
`/private/tmp/pinfold-worktree-research-20261015` (`B`). They intentionally
refuse or fail on existing fixture directories rather than resetting them.
The original run's scratch directory was removed after evidence capture.
Require a fresh `B`, then copy the scripts/results manifest into it. The
commands below are a reconstruction recipe, not an instruction to change the
primary checkout or installed Pinfold.

```sh
B=/private/tmp/pinfold-worktree-research-20261015
# E must name your copy of this evidence bundle; B must not already exist.
mkdir "$B"
cp "$E"/*.py "$E"/source-manifest.json "$B/"
git clone --no-hardlinks <checkout> "$B/source"
git -C "$B/source" checkout --detach 198600b1751d889d354ed3657d8f839e8436e469
git -C "$B/source" apply "$E/baseline.patch"
(cd "$B/source" && ZIG_GLOBAL_CACHE_DIR="$B/zig-global" ZIG_LOCAL_CACHE_DIR="$B/zig-local" cargo build --release --locked)
hdiutil create -size 2g -fs 'Case-sensitive APFS' -type SPARSE -volname pf-wt-research "$B/casesensitive"
mkdir "$B/cs-mount"
hdiutil attach "$B/casesensitive.sparseimage" -mountpoint "$B/cs-mount" -nobrowse
```

The installed default profile image must exist. Its observed image ID was
`d44cb091aee72c3630d5fcf1077a0ca47ca422c0df718a5a7fc909235ef75c98`.
The scripts use its tag and record the resolved ID in Pinfold results.
They use cached images, no egress, synthetic repositories and 2 CPUs.

Run via `UV_CACHE_DIR=/private/tmp/pinfold-wt-uv-cache uv run --no-project SCRIPT`.
In this order:

1. `case_probe.py`, `case_followup.py`, `ancestor_probe.py`, `deep_probe.py`.
   These share the disposable case-sensitive fixture. The first script's
   `preexisting_hardlink` label means a link **introduced by the host after
   startup**, not a pre-start admission test. The raw label is preserved.
2. `migration_probe.py`: separate case-insensitive main metadata, one-time
   working-file relocation, `git worktree repair`, exact dirty/index checks.
3. `isolated_probe.py`, `core_boundary_probe.py`, `apply_probe.py`.
   The `.GIT` row in `apply-probe.json` stopped on an invalid synthetic gitfile;
   it is **inconclusive**, not evidence of protected-path rejection.
4. `copy_benchmark.py`, then `runtime_benchmark.py`, `storage_probe.py`, and
   `import_benchmark.py`. Each must finish before the next. The fixture is
   100,000 4 KiB files, one 64 MiB file and a manifest, 476,708,872 logical bytes.
   The small source subset is 95 files / 1,291,148 bytes. Exclude build outputs.

Native `container run -d` timings are labelled `started`, not Pinfold-ready.
Pinfold timings wait for its actual `ready` JSON. Work measures a root edit
and full file enumeration, not model latency or compilation. First trials
are first-use observations with cached images and uncontrolled OS cache;
none is a reboot-cold benchmark. Large-tree timing is synthetic and does not
stand in for every real monorepo. Runtime benchmark native pointers are
synthetic; Git semantics were tested separately in real repository probes.

## Linux

The original run used a leased disposable Debian 13.7 x86-64 VM, ext4,
kernel 6.12.107+deb13-cloud-amd64. Git was 2.47.3. Podman was unchanged
upstream 6.1.4, source `464c15fec06f0074c916f9eff87c4806869c2833`, binary
SHA-256 `11326740634a0929f438256c4e3ff90273c4399a279c68e9fde02be25f75c2a5`.
The retained build was transferred from the prior upstream build evidence;
its SHA-256 was rechecked. Debian's installed 5.4.2 was not used for the
reported native probes. A rootless systemd user session was enabled only
inside the lease. See the current repository's `scripts/setup-linux-runner.sh`
for the upstream native build procedure; avoid changing an existing server.

In the disposable VM, install Git/Python and rootless Podman prerequisites,
provide the 6.1.4 build and compatible storage configuration, and set
`XDG_RUNTIME_DIR=/run/user/1000` and
`DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`.
Build `localhost/pinfold-wt-research:probe` from:

```dockerfile
FROM docker.io/library/alpine:3.23
RUN apk add --no-cache git
```

Observed base ID:
`f6d31b983274a75183386cda3aac54849498fac0fc3e0d98f92ff56ed517d92b`.
Observed built image ID:
`a1c4e472995c510926cf9c3b9ec297a08509bb41028c5aa14c8a2449e144c922`.
This recipe is not a bit-reproducible image pin; repeat runs must record their
actual image/package versions. Create `/tmp/pinfold-wt-probe`, then run
`linux_probe.py`, `linux_benchmark.py`, `apply_crash_probe.py` and
`apply_race_probe.py` with Python 3, sequentially. All fixtures use `/tmp`.
The last two use inotify events, not sleeps, to observe an interrupted apply
and an ordinary concurrent host save. Git apply is the real installed tool.

`linux_probe.py` used default ten-second graceful removal, recorded in its
result. The lifecycle benchmark uses forced zero-grace removal and records
that separately; do not attribute the ten-second wait to normal Pinfold
teardown. Linux results are native Podman, not an integrated Pinfold worktree
candidate or a full Linux security suite.

## Matchlock and evidence limits

`filesystem-components.md` gives pinned sources and the exact successful Go
command. `matchlock/main.go` imports the actual provider from that source.
Its `go.mod` expects a checkout at
`/private/tmp/pinfold-research-matchlock-20261015`; clone the pinned commit
there or update only that local replacement. Isolate Go caches as documented.
This is a native host-component probe, not a whole-VM run.

`alternatives.md` records other primary-source comparisons. The independent
review is chronological: earlier findings are followed by the final consistency
review, which records what later evidence resolved. The main report is the
current conclusion for this dated investigation.

## Cleanup

Every runtime script names its own containers and removes only those names.
If interrupted, inspect matching labels (`dev.research.owner=worktree-20261015`)
and actual ownership before removing anything. Never prune a shared runtime.
Copy results out before releasing the Linux lease. On Mac, verify no owned
containers/processes remain, verify the image-to-mount association, detach
this exact disk image, then remove only this experiment's scratch roots.
Pinfold's stable per-name lock files are intentionally never unlinked. Their
exact paths and absent owner/socket records are in `verification.json`.
`cleanup.json` records the original run's final verification.
