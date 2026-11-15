# Login files from routes: PR 3

2026-11-15. Implementation commits `c74710b` and `a151168`, from `a99c3be`. No push, PR, release, product dependency or suite test. [Collectors and retained output](2026-11-15-login-files-from-routes/). The owner approved correcting Linux's never-working 24-attachment limit to the measured ceiling of 14. The corrected branch's boundary control is pending.

Routes generate Claude's managed settings, Codex's system provider and Pi's single autoload subdirectory. Renderers receive metadata and placeholders only. HOME, collisions and attachment counts validate before preparation. CLI/attach/Pi launch image executables through PATH. PINFOLD_ALLOW remains deliberate. Artifact reporting uses the host helper's architecture and executable. README/spec describe the contract; the installer-manifest comment was updated. Profile seeds stay unchanged.

The approved correction is implemented exactly: mark disks unclean and record attachments before preparation writes; populate fresh disks once, as the numeric box user; then create missing generated parents; then attach generated configuration. Reused disks are neither repopulated nor chowned. Parent creation uses descriptor-relative no-follow traversal after privilege drop. Existing ordinary parents are tolerated; symlinks and non-directories fail. Apple uses the networkless transient helper under the disk claim. Ordinary guest capabilities are unchanged. The order is stated beside the lifecycle wording in ARCHITECTURE.md.

Deleted: harness declarations/validation/membership, credential-env metadata, HARNESS_ENV/harness_env/harnesses.json, downloads/mounts, per-exec defaults, forced Pi extension arguments, launch-record versions, guest pins and unused guest updater adapters. Infrastructure pinning, the verified installer, Install::Executable, needed npm adapters, harness-exited, crossing, scratch handling and operating-context instructions remain.

Caller changes: hard removal of `harnesses`, `--harness`, launch-record `harnesses`, guest artifact entries and mounted `/opt/pinfold/<agent>` executables. Artifacts reports host helper/infrastructure, with the helper executable path. Login routes alone create native files. Pi requires writable absolute HOME. Generated-path collisions refuse. Linux now admits at most 14 disks and mounts, including generated mounts; 15 refuses as `spec`. There is no migration path.

Reproduction: set P to the built binary and IMAGE to `localhost/pinfold/profile-default:latest`; run `validation.py "$P"`, `accept.py`, `gates.py "$P"`, `extra.py`, and on Linux `capacity.py`. Use the isolated paths/maintenance stamps in the collectors. Linux used a fresh three-hour Debian 13 lease, only `labvm exec -- bash -lc`, a source patch on the origin clone, musl-tools/rustup/build-essential and zstd for actual compressed fixture requests. Long jobs used setsid/nohup. Boxes used two CPUs and 1 GiB. The lease was not released.

| Case | macOS | Linux |
|---|---|---|
| Fresh .pi/agent/settings source plus Pi route; reused content/mode/ownership | Passed | Passed |
| Symlink/non-directory refusal; no target write; disk-unclean refusal; same-home correction | Passed | Passed |
| Every Pi provider, Claude/Codex tool exchanges, child agent, synchronized Codex/Pi | Passed | Passed |
| Read-only generated files, writable siblings, sibling extension, route rename/removal | Passed | Passed |
| Explicit image without profile, no passwd entry, nonstandard HOME, readonly ancestor/writable child | Passed | Passed |
| HOME/collision/legacy refusals and corrected controls; bare-box PINFOLD_ALLOW | Passed | Passed |
| Stored/explicit keys, project overrides, helper architecture, TTY, Pi/attach, cancellation, marker scans | Passed | Passed |
| Attachment 15 refusal / attachment 14 startup | Linux-only | Corrected-branch rerun pending; baseline 14 ran and 15 failed before init |

The gates passed with Pi 1.5.2, Claude 2.1.311 and Codex 0.169.0 on both hosts. Pi autoloads the named subdirectory. Stored keys win its placeholder; project extensions and explicit keys were exercised. Claude managed env wins user/project/explicit settings and inherited env. Codex user/explicit config overrides system config, but this release ignores project-local model_provider. The native diagnostic is retained. Gate HTTP 400 establishes selection only; accept.py separately requires successful tool results, guest files and HTTP 200 exchanges. A hostile project bypass fixture received no host authentication. No real login was read or used.

[checks.txt](2026-11-15-login-files-from-routes/checks.txt) records build/fmt/Clippy, native helper architecture, static ELF and matching source content identity. Mac embedded the ARM musl init. One Linux default-profile build took 209 s; no Mac profile build was needed. The Linux-only version accessor compilation fix is a151168. Idle-runtime startup with a fresh home and all three generated mounts measured 2.267 s on Mac and 5.427 s on Linux. These are single observations.

The original 24-attachment case failed: the pinned Firecracker x86 MMIO allocator cannot attach that many disks/mounts. An external trace captured “Could not attach device: MMIO transport error: Failed to allocate requested resource”. A real 14-attachment box execs successfully; 15 and 24 fail before BOOT. The actual `origin/main` baseline `a99c3be` also starts and execs at 14, while 15 fails before init with VMM exit 1. This matches the [pinned runtime's 17-device ceiling](https://raw.githubusercontent.com/firecracker-microvm/firecracker/v1.17.0/CHANGELOG.md), with init, root and vsock consuming three devices. This is not VM RAM/disk exhaustion.

Owner decision A corrects the admission limit and guarantee 17 to 14, with 15 refused as `spec` before preparation. The manual collectors now exercise that boundary. The change corrects a limit that also failed on the main baseline; it changes no transport, PCI setting, kernel, pin or privilege. The corrected branch's boundary run is pending. All other original requested cases ran and passed; live subscription/provider compatibility was excluded and remains unverified.

All owned boxes/disks were removed. Linux had 22 GiB disk free and nearly all 2 GiB tmpfs free. Mac ended at 16 GiB free; every build started above 12 GiB. No snapshot was deleted. The Linux default image and its build cache remain; no Mac image was created. The verification directory consolidates the earlier gate/blocker evidence into current collectors and compact text output; historical details remain in commit 0f7be4f.
