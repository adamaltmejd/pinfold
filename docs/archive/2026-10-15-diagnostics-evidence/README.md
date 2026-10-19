# Diagnostics research evidence, 2026-10-15

Omitted from this archive copy: the source patch and manifests, binary hashes, logs,
baseline and prototype results, cleanup receipts, and the `prepare.py`, `probe.py`,
`prototype-probe.py`, `human.py` and `cleanup-check.py` drivers.

These are disposable research artifacts, not a supported API or production patch.
The parent [report](../2026-10-15-diagnostics-architecture.md) defines the conclusions and limits.

- `working-tree.patch`, `source-baseline-reconstructed.json`: inspected dirty source baseline. The initial manifest receipt was overwritten by the baseline probe; reconstruction reverses `prototype-b.patch` from the isolated source and matches all tracked files. No historical untracked-content claim.
- `binary-hashes.json`, build/clippy/fmt logs: built artifacts and checks. Cross-build linker warning retained; no full-suite claim.
- `baseline.json`, `prototype.json`: macOS CLI probes. `linux-baseline2.json`, `linux-prototype.json`: Linux equivalents. `linux-setup-refusal.json`: initial isolation error, not product regression.
- `consumer.ts`, `consumer-run.ts`, `consumer.json`: repaired direct subprocess adapter and finite-output/lifecycle cases. `consumer-first.json` needed manual exact-name orphan cleanup after owner death because inherited stderr prevented EOF; fixed version explicitly reports a drain deadline.
- `pi-baseline.json`, `pi-prototype.json`: real Pi RPC readiness, host attach and changed-config experiments. `human.py`, `human-output.txt`: host renderer applied to the real retained Pi record.
- `edge.json`: macOS public CONNECT, bounded query and unexpected runtime termination; non-reading output was too small to block.
- `linux-edge.json`: same source on Podman, including real 4 KiB pipe backpressure and observer cancellation. This case actually blocked.
- `prototype-b.patch`, `prototype-b-notes.md`: scratch binary additions and omissions. No production adoption intended.
- `comparators.md`, `consumers.md`, `independent-review.md`: current source/documentation research and independent review.
- `local-cleanup-before.json`, `linux-cleanup-before.txt`, `linux-cleanup-final.txt`, `linux-locks-final.json`, `cleanup-final.json`: scoped cleanup evidence. Stable lockfiles remain by spec; no live research ownership remains.

## Reproduction

Requires the recorded dirty source, supported real runtime, existing `pinfold/profile-default:latest`, Rust/zig tooling, Python through `uv run` locally, and Bun for the consumer. Do not run on an uncoordinated shared host. Do not copy production secrets into these fixture environments.

The drivers use an explicitly owned `/private/tmp/pd-1015` on macOS or `/tmp/pd-1015` on Linux, private XDG roots, unique named boxes, a host HTTP fixture and exact-name cleanup. They set their own private maintenance timestamp to suppress unrelated host-wide automatic cleanup; that internal-file accommodation is research scaffolding, not part of the proposed public integration API. Use a new path/name prefix for a new investigation.

1. `prepare.py` copies tracked current source and records hashes/status/diff. Its receipt filename is corrected to `source-baseline-initial.json` to avoid the collision discovered in this run.
2. Build baseline with `CARGO_TARGET_DIR`, `ZIG_GLOBAL_CACHE_DIR` and `ZIG_LOCAL_CACHE_DIR` under the owned scratch tree; `cargo build --locked --bin pinfold`. Copy the binary as `pinfold-baseline` before applying the prototype patch.
3. Run `uv run probe.py` against the baseline. `PD_BIN`, `PD_ROOT`, `PD_TAG` select binary/root/receipt. Real CLI refusals and success controls are retained, not prose asserted as expected values.
4. Apply `prototype-b.patch` with `patch -p1` in the copied source. Build the same way; copy as `pinfold-prototype`. Run `prototype-probe.py` with `PD_BIN` and `PD_TAG=prototype`. Run `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings` in the scratch source.
5. Run `bun consumer-run.ts`; run `pi-probe.py` with baseline and prototype binary/TAG pairs. `edge-probe.py` uses Apple Container; `linux-edge-probe.py` uses Podman and an actual smaller OS pipe.
6. For Linux cross-build: `cargo zigbuild --locked --release --bin pinfold --target x86_64-unknown-linux-musl`. This run used installed 0.2.2 only for Linux baseline, then the newly built 0.2.3 scratch binary for prototype. Preserve the host's existing `CONTAINERS_STORAGE_CONF` when isolating XDG config; do not write a new shared storage configuration.
7. Verify exact-name box absence, released ownership locks, no probe processes/mounts and private-directory removal. Never unlink stable Pinfold coordination locks or prune unrelated runtime resources.

The recorded timings are individual observations; differing images, binaries and shared-host activity preclude performance comparisons. Snapshots/stat output prove only their stated observations. No full security, privacy, crash-durability, release, Linux-arm64 or remote-service validation is implied.
