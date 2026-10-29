# Host builder verification with the existing Ring dependency

Date: 2026-10-29. Bounded candidate from
`db3fef58dd7a7625156430c9920b75b273fb09ff`. Adam's latency direction
authorizes reducing measured host cost while retaining security controls.
This change uses Ring only for `core/artifacts.rs::builder_file`'s
streaming SHA-256. It neither skips verification nor caches a prior
verification result.

The operator reported these unchanged-source/component observations:

| Observation | Time | Scope |
|---|---:|---|
| Normal tiny COPY, first cold build | 30.83 s | Whole ordinary CLI build, before this candidate. |
| Normal cached build | 12.08 s | Whole ordinary CLI build, before this candidate. |
| Builder artifact verification report | 4.7576 s | Ordinary artifact CLI, before this candidate. |
| Ring release scan of the pinned 1 GiB root | 2.6488 s | Independent component scan, AVX2 x86 CPU without SHA instructions; expected `ef1aaa0f…` matched. |
| Ring unoptimized dev scan | 14.439 s | Component regression; motivates the package's dev optimization override. |

These figures are operator-reported inputs to this candidate, not new
measurements performed for this commit. The component timing is not an
integrated CLI comparison, guest speedup or CI-budget result. In particular,
subtracting it from artifact-command time would not establish a saved
duration. The changed normal CLI still needs measurement and integrity
validation after source review.

The read/copy loop remains 64 KiB. Open-file type/length, cumulative size,
growth/short-read refusal, pinned expected digest, lowercase hex, deadline
and cancellation checks are unchanged. The same bytes feed the hash and
optional create-new 0600 snapshot. Only SHA-256 context construction and
digest finalization change. Every installed-root use still verifies its
complete pinned bytes. Bootstrap identity, installation, publication and
all guest/converter behavior are unchanged.

Ring 0.17.14 is already locked and used by the host's TLS backend. The new
direct workspace/Pinfold edge requests no features. Offline Cargo metadata
before/after shows identical package IDs and feature maps; only Pinfold's
direct dependency node changes. Ring still has `alloc`, `default` and
`dev_urandom_fallback`. Cargo.lock adds only that direct edge, with no new
package, version or checksum. `[profile.dev.package.ring] opt-level = 3`
avoids knowingly selecting the regressing unoptimized component profile;
its integrated effect remains unverified.

Other Sha2 uses, including `artifacts::init`, stay unchanged. Standalone
`pinfold-init` still uses its existing pure-Rust hashing path; its resolved
dependency closure reaches neither Ring, rustls nor `cc`. Its manifest,
guest source and cross-build script are unchanged. No new C cross-build
requirement enters that target. No hash wrapper or wider migration is added.

Static security/dependency reviews and formatting completed. Only offline
Cargo metadata and the formatter ran: no compilation, test, VM, public push
or release was performed for this candidate. Runtime correctness, actual
cold/cached latency and the unchanged full-suite budget remain open gates.

Metadata snapshots are retained under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`:

| File | SHA-256 |
|---|---|
| `builder-ring-metadata-before-db3fef5.json` | `4e600ca4494bb646352555d61deda873475d6524a827850a0abeaeb465b3f468` |
| `builder-ring-metadata-after-candidate.json` | `2c83aa157b2624e502496bf44ea5368df7cf431036756f7975f0f92d7b6f4692` |
