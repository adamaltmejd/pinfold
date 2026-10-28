# Builder integration source candidate

Date: 2026-10-28. Baseline: `ff6276cb832dfc78e4cb716857e33af1f00f12b8`.
Branch: `codex/issue230-builder-consumer`.
Worktree: `/Users/adam/.codex/worktrees/issue230-builder-consumer/pinfold`.

Adam chose the cohesive Linux replacement after the cut audit. He also
approved one shared public base rule: only a managed project's selected
local profile identity occupies `dev.pinfold.base` and the build's base
field. Caller/profile builds leave it empty on both platforms. Other used
source digests stay internal. This explicit decision supersedes the prior
requirement to preserve first-FROM output. Other Apple policy stays intact.

## Source boundaries

- Embedded bootstrap pins and a documented ordinary offline archive cache;
  bounded verified staging and raw-root identity checks before use.
- Separate trusted guest builder/converter entry point, unchanged outer
  jail, fixed read-only inputs, exclusively claimed cache, fresh converter,
  production proxy/audit, independent explicit build additions.
- Typed bounded output, complete OCI graph/config/label admission, explicit
  attestation/runnable separation, internal actual-source provenance,
  completed opaque root receipt and one atomic publication generation.
- Leased local bases and box roots, no host recipe parser on Linux,
  no host registry auth, no host archive unpacking or writable-disk read,
  no first-launch conversion. Old Podman storage/disks are not migrated.
- Read-only catalog/allocated cache inspection and exclusive idle-cache
  cleanup. Short generation-owned sockets use the common checked namespace.

The source reuses serde, serde_json and sha2 already in the workspace for
the x64 guest helper. No new crate or external build tool is introduced.

## Gates still open at this source checkpoint

This is a review candidate, not production acceptance. No merge, release,
tag, public artifact upload or permanent-infrastructure change is admitted.

Numeric choices in candidate code are not approved policy: 512 MiB inputs,
payload/root output, 1 MiB JSON, 8 KiB headers, 4096 objects, two vCPUs,
2560 MiB RAM, 8 GiB cache, fresh 3 GiB converter storage and five-minute
VM jobs. These are trial-derived allocations or conservative test
candidates. They require externally observed boundaries/fit, tradeoffs and
Adam's final numeric decision. They are deliberately absent from the
binding spec. Bootstrap installation keeps its existing download contract.

Fresh public bootstrap installation is unavailable. The normal offline
cache uses the actual preserved archive pinned in `builder.toml`, not a
self-authorizing manifest or a locally rebuilt identity. Public immutable
distribution needs separately authorized publication and a reviewed pin
to the emitted bytes. The cut record gives the concrete workflow plan.

At this checkpoint, locked all-target Clippy passed on Linux and macOS,
and the Linux musl binary built. Runtime attempts have not passed:

1. The first real CLI attempt refused an overlong Unix proxy socket before
   starting a VMM. Socket placement now uses the short common namespace.
2. The next all-target check caught a Linux-only unused test import; it
   prevented that script's smoke step. The import is fixed.
3. The next real CLI attempt reached input formatting but mkfs failed
   because its role-state parent directory was not created after the socket
   relocation. The source now creates it before formatting.

Raw local diagnostics are retained under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
No builder, converter, agent-reuse, malformed-output or suite success is
claimed here. Full Linux/KVM and exclusive Mac suites, the suite budget,
capability/ACL fidelity, failure/recovery/concurrency evidence and final
independent correctness/security plus lean/spec/test review remain gates.

Malformed headers and hashes cannot honestly be exercised by replacing the
pinned guest with a stub or injecting a synthetic host stream. Use genuine
normal-seam interruption/corrupt-output evidence where possible; report any
fault-injection boundary not reached rather than calling the scratch model
an integrated test.

## Caller changes in the candidate

- Linux build egress is allowlisted; `build` and `image build` accept exact
  repeatable `--allow HOST` additions, without host registry credentials.
- Linux identity is the runnable manifest; conversion completes before
  successful publication and launch requires completed preparation.
- Old-store-only Linux refs stop resolving; rebuild explicitly. Old Podman
  storage and managed disks remain untouched.
- The approved shared base-field rule applies on both platforms.
- Bootstrap cache misses are explicit until distribution is resolved.
- Final resource limits and refusal behavior remain a policy gate.
