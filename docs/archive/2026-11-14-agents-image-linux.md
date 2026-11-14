# Linux agents-image probes blocked by image preparation

2026-11-14, ticket #298 probes 5 and 6. Linux source `277b436`, fetched
from `probe/agents-image`. Runs used the existing trial lease, through
`labvm exec -- bash -lc '...'` only. Evidence ran from 19:01 to 19:16 UTC,
before the brief's 00:28 UTC cutoff. The lease was neither acquired nor released.
No Mac runtime command or real credential was used.

The combined image did not become runnable. The default build refused
`nodejs.org`. An explicit build-host addition passed installation, but
conversion exhausted its work disk. An unchanged rebuild reproduced that
failure. No image was published. The dependent agent probes could not run.

Evidence: [commands and selected raw output](2026-11-14-agents-image-linux/results.txt),
[every audited host decision, grouped with counts](2026-11-14-agents-image-linux/build-egress.tsv),
[x64 recipe](2026-11-14-agents-image-linux/Containerfile), and
[manual collector](2026-11-14-agents-image-linux/probe.py).
Output excerpts omit repeated progress and diagnostics. Audit aggregation
omits timestamps only. Paths and the username are normalized to `user`.

## Build results

| Case | Command or recipe | Elapsed | Result |
|---|---|---:|---|
| B1 | Original Mac Containerfile; `pinfold build --profile probe-agents` | 24.364 s | Exit 1. Node checksum GET received proxy 403; audit says `not allowlisted`. |
| B2 | x64 Containerfile; add `--allow nodejs.org` | 139.924 s | Exit 1. Installation completed; converter failed writing `/work/root.tar`: `no space left on device`. |
| B3 | B2 plus a size-reporting RUN ending `exit 1` | Not timed | Deliberate recipe failure. Captured installed file sizes. |
| B4 | B2 unchanged; same explicit build-host addition | 117.666 s | Exit 1. Same converter failure. |
| B5 | B2 plus metadata/size RUN ending `exit 1` | 9.465 s | Deliberate recipe failure. Bounded failure tail omitted early metadata. |
| B6 | B2 plus short metadata RUN ending `exit 1` | 7.683 s | Deliberate recipe failure. Captured cached install timestamp and versions. |

The only final-recipe change from the Mac recipe is
`linux-arm64.tar.xz` to `linux-x64.tar.xz`. Node's source remains `nodejs.org`.
The per-build `--allow nodejs.org` uses the existing Linux interface. Neither
the spec's list nor product code changed. Diagnostic RUNs are not in the
retained final Containerfile. The isolated profile was restored afterward.

B2 was the first agent installation with an empty named npm cache; this
was not a cold builder-store measurement. The VM already had a builder cache
and a different profile image. B1 also warmed Debian layers. B6 reports npm
cache growth from 4 to 451588 KiB. Each build's host-disk preflight showed
21G or more in `df -h`; the exact free-byte check also passed 12 GiB.

There are no successful cold-build or unchanged-rebuild times. The table
reports failed attempts. There is no completed OCI catalog image or runnable
root whose size can be reported. In the builder, `du -sk /usr` reported
1344060 KiB and `/var` 7560 KiB. These are installed-tree measurements,
not compressed OCI size, completed root size or unique host storage cost.

The unchanged failed rebuild did not re-resolve `latest`. B4's complete
audit contains only Docker registry/auth decisions. B6 still reads the
install timestamp `2026-11-14T19:05:24Z`, from B2. Its cached package list is
pi 1.5.2, Claude 2.1.325 and Codex 0.169.0; build RUNs report Node 24.22.3
and npm 11.19.1. This establishes cached installation reuse through these
failed builds. It does not establish a successful rebuild or fresh metadata
when installation is explicitly rerun.

## Audited build hosts

These are all distinct names in all six build logs. An `allowed` decision
records proxy authorization; it does not by itself prove upstream delivery.
The TSV retains every distinct host/decision/reason and its exact count per run.

| Host | Allowed decisions | Refused decisions | In spec's initial Linux list? |
|---|---:|---:|---|
| `auth.docker.io` | 6 | 0 | Yes |
| `registry-1.docker.io` | 6 | 0 | Yes |
| `deb.debian.org` | 74 | 0 | Yes |
| `registry.npmjs.org` | 17 | 0 | Yes |
| `nodejs.org` | 2 | 1, `not allowlisted` | No; explicitly added for B2–B6 |

The complete initial list in [Linux image preparation](../ARCHITECTURE.md#linux-image-preparation)
is `registry-1.docker.io`, `auth.docker.io`,
`production.cloudfront.docker.com`,
`docker-images-prod.s3.dualstack.us-east-1.amazonaws.com`, `deb.debian.org`,
`github.com`, `release-assets.githubusercontent.com`, `registry.npmjs.org`
and `cli.github.com`. Only `nodejs.org` among observed names is missing.
Cached base layers mean these attempts do not establish a complete host list
for a cold store, other recipes, native installers or runtime updaters.

## Preparation limits and owner decision

B2 and B4 reproduce the converter scratch failure through the real CLI.
The [host job](../../crates/pinfold/src/core/image/linux/job.rs) fixes
the converter's work disk at 3 GiB. The converter's BuildKit cache and
`root.tar` share that disk. This explains why the guest can exhaust space
while the VM host still has about 21 GiB free.

There is a second, source-derived blocker, not a second observed runtime error.
[Guest conversion](../../crates/pinfold/src/init/builder.rs) caps the
converted root at 512 MiB, including filesystem overhead. B3 measured
Claude's main executable at 365842960 bytes and Codex's at 294277432 bytes:
together 660120392 bytes, or 629.54 MiB, before Node, pi, npm, libraries
or filesystem overhead. The two Claude paths in the excerpt are not summed.
Keeping these executables cannot meet that root bound merely by deleting docs.

The owner needs to choose whether to change Linux preparation's resource
bounds or use a different installation design. Adding Node's host alone is
insufficient. No implementation, security control, agent version or test changed.

## Dependent cases, retained state and checks

S1 records `box up` refusing the requested image as `image-missing`, exit 1.
Consequently none of the following is established on Linux: the three agents'
`--version` or normal no-login starts from this image; their self-update
results or errno; npm pi's CONNECT refusal against a login-route fixture.
The Mac findings are not used as Linux results. Separate agent images and
mount-based substitutes were not run.

The host was Debian 13.7 x86_64, kernel `6.12.107+deb13-cloud-amd64`, with
rustc 1.99.1. The musl binary was built with
`cargo build --locked --target x86_64-unknown-linux-musl`.
The collector used `/tmp/pfl298` for XDG config/state, a fresh maintenance
stamp, the real cache, `PINFOLD_NO_UPDATE_CHECK=1`, `PINFOLD_CPUS=2` and
`PINFOLD_MEMORY=1G`. The refused box spec also names 2 CPUs and 1G.

Final inventory: no `probe-agents` image refs, no probe boxes, no disks in
this state root, and no pinfold or Firecracker process. No image was created
to leave. The earlier `probe-linux294` image's refs and identity were unchanged.
The shared builder/npm cache and isolated scratch evidence remain on the
leased VM; expiry will erase them. No lease operation was performed.

`cargo fmt --check` and Linux musl
`cargo clippy --all-targets --locked --target x86_64-unknown-linux-musl -- -D warnings`
passed. Python parsing, evidence/privacy checks and `git diff --check` passed.
No tests were added or run. Changes are this record, the x64 recipe, the manual
collector and selected output/audit files. Nothing was pushed or released.
