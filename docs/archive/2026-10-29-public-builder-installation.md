# Public builder installation

Date: 2026-10-29. Production source is
`3c9f9d6c90913c562ca7eb504266007730c93ff4`, with executable SHA-256
`a9e7604cb7d7122d080c6e8f76e9e1281e009dc4bcccd726ccad6fb50c78ce6e`.
This is a normal CLI trial on the exclusively leased disposable Debian
Linux VM, not full Ubuntu CI, suite-budget admission or numeric approval.

## Published bytes and consumer

The approved exact bootstrap is published as the non-latest
[`builder-root-09393a02…` prerelease](https://github.com/adamaltmejd/pinfold/releases/tag/builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4).
The operator independently verified the twelve assets and actual tag target
`5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f`; the publication worker verified
all twelve public downloads. GitHub does not enforce release immutability
for this release. Embedded digests remain the consumer's authority.

The consumer adds the exact HTTPS archive URL without changing its archive,
raw-root or recipe identities. An absent installed root uses the documented
offline archive when present, otherwise the existing HTTPS downloader.
The existing byte bounds, shared installation deadline, cancellation,
archive and raw-root verification, fixed root output and atomic directory
installation remain. `artifacts` and `doctor` include the URL. No runtime
role, image catalog, dependency, resource policy or user configuration
changes.

## Fresh-cache trial

The external observer used the production executable as uid 1000 with
separate empty XDG roots and a caller recipe `FROM scratch` plus `COPY`
of a host fixture. It supplied no archive, installed bootstrap or VMM.
Strace recorded actual curl execution of the pinned URL. Inotify observed
the completed downloaded archive; the observer held a read-only descriptor
and copied those actual bytes before staging removed the path.

| Case, with identical CLI arguments | Observation |
|---|---|
| Transport denied by a closed loopback HTTPS proxy | Exit 1, failed line `download failed: exit status: 7`, 0.049 s. No bootstrap install or changed public refs. |
| Normal environment, still no seeded archive or root | Built/exit 0 in 49.621 s. Captured archive is 122,578,909 bytes, SHA-256 `09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`. |
| Installed builder removed; one byte flipped in the same-size offline archive | Exit 1, `builder-integrity: checksum mismatch`, 0.835 s. No fallback URL execution, installed root or public-ref changes. |
| Verified captured archive restored at the offline path | Built/exit 0 in 36.413 s. No bootstrap download. |

Both successful installations have a 1,073,741,824-byte root with SHA-256
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
Artifact reporting changed from uncached to verified cached and retained
the exact URL. The corruption case proves the shared verifier through its
offline input; it does not simulate corrupted bytes from the public server.
The observer's umask and private fixture directories do not establish a
general staging-permission guarantee.

## G44 scenario and test audit

Public download makes moving the cached bootstrap alone insufficient to
prove builder-independent image launch. The existing G44 scenario now moves
its private builder directory aside and puts a regular file at that path.
An unexpected installer cannot descend through it. Cached VMM artifacts
remain available. After the existing successful launch, fixture read and
teardown, the file is removed and the builder directory restored.
`TestEnv::drop` removes the entire private fixture on panic, including the
saved directory. Shared suite artifacts are untouched. No network override,
new test, assertion, helper or production seam is added.

The four test-audit answers are: existing guarantee 44; the comment names
bootstrapping during image launch as the sabotage; the harder scenario
blocks both cached reuse and new download, where moving bytes alone no
longer does; expected readiness follows the existing launch contract and
the existing read compares bytes from the caller fixture. The earlier
successful role build remains its positive control.

The 25-line private-cache offline-copy setup in `default_image` is removed.
It assumed a shared archive directory existed even for a standalone test.
Each private cache now uses the same real profile build and production
download as a user. Successful-build tracking and all assertions remain.

The updated G44 passed in **168.23 s**, with matching before/after production
executable hashes. Its invocation also passed formatting and locked
all-target Clippy. The invoked test files have SHA-256:

- `crates/e2e/src/lib.rs`: `b2e3602bd516d8a69cbe9dbabbc5c71fe002da2ad1ef6f23e8f7b2fcd2cec7e1`.
- `crates/e2e/tests/e2e/box_.rs`: `8e317ae2882153c95c80dab930b8317c318e25561835f3d92cf41fa08a6f688a`.

Both bounded source reviews passed, including the test-audit questions and
the removal of obsolete offline seeding. Final documentation changes do
not alter the production source or builder pin tested at `3c9f9d6`.

## Evidence

Local evidence is under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
The download bundle contains the actual public archive, observer source,
curl exec traces, CLI stdio, command timing, catalog snapshots and root
receipts. It excludes guest-writable caches and disks.

| File | SHA-256 |
|---|---|
| `public-download-source-3c9f9d6.tar.gz` | `f868e8ff13876ee4e8adc5cc80b9c0a7ee1fa4cc84f2cc99fb8a5103358aa34c` |
| `public-download-proof-3c9f9d6.tar.gz` | `bb7292ee68169591e3f06216c5dcc73761a541ad3cf6196379d264880e241a42` |
| `public-download-proof.py` | `29a100965f2c7bf9206ebc5b14a8e10f97a7049ffdf6c37941d8cb99655a53e5` |
| `linux-public-download-build-1.log` | `7b9b9f690f1505dd90f7628b4dd277be0d25b8c993dfcace4dfff4b9c723e4da` |
| `linux-public-download-proof-1.log` | `50dae50804b324a2420956463809e0b46319a3f72d2ae7ff77b1b92d2ec9c974` |
| `linux-public-g44-1.log` | `4b30a704f8ea9d67344bfeb3d97fa38407b1ae3a636e0f250d9868902a2940a2` |

Linux formatting, locked all-target Clippy and musl build passed. Mac
formatting and locked all-target Clippy passed with a task-owned Zig cache
after the default cache was sandbox-denied. The initial disposable-VM setup
stopped at an existing package-manager lock; normal lock waiting completed
setup before these runs. Neither setup failure was a product result.

Full Ubuntu CI, its five-minute budget, remaining runtime boundary evidence,
numeric policy and operator admission remain separate gates. This note
does not authorize runtime merge, a CLI tag or release.
