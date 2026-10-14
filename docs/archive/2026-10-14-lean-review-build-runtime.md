# Independent implementation review: build/runtime changes

Reviewed the initially clean worktree `/Users/adam/.codex/worktrees/pinfold-lean-implementation/pinfold` at exact HEAD `a7495da045010d9dcabd8f944a0342dc9782d9b4`, base `a1be921`. Reviewed the individual commits `a00161c`, `1a9281a`, `98c1b7f`, and `a7495da` and their cumulative context. Read AGENTS.md, the binding ARCHITECTURE controls, test-audit, and the affected source/manifest/lock/spec/test files whole. Read ownership, directory installation, host entrypoint, and release workflow context where needed. At the final state check, a concurrent README-only commit advanced HEAD to `01563845f9b915c2495301e45e986e58630f7e6d`, and an investigation README was untracked. Verified the intervening commit changed only README.md; none of the reviewed files changed.

No concrete blocker found in the four assigned commits. This is source review, not compilation, runtime validation, or performance evidence. No builds, tests, runtime commands, or source edits were performed. Other commits visible in the cumulative diff (O1, O4, B1, B2, C3) are not independently approved by this report. The unrelated seccomp change is excluded.

## O2: build-time digest (`a00161c`)

- `crates/pinfold/build.rs:68-76` copies the nested build output into the exact file later consumed by `include_bytes!`, then hashes those copied bytes. Both the path and digest are emitted by the same successful build-script run. There is no digest of a different source or compiler input masquerading as a payload hash.
- `crates/pinfold/src/core/artifacts.rs:192-208` retains the same content-addressed directory, installed-file manifest check, atomic installation path, missing-file repair, and executable mode. It removes deterministic hashing of embedded bytes, not verification of a downloaded or mutable payload. Cache content integrity is not newly promised; the pre-existing installer checks existence, not warm-cache byte hashes.
- `build.rs:19-26` uses TARGET, not HOST. Non-Apple builds emit a placeholder and empty digest, but `artifacts::init` returns the executable before either is used. macOS-hosted Linux builds do not recurse or invoke Zig through this script.
- The build dependency is the existing workspace sha2 version. Cargo.lock adds no registry dependency for O2.

## O6: dedicated shared init (`1a9281a`)

- `crates/pinfold-init/src/main.rs:1-2` compiles the actual existing init module. `crates/pinfold/src/init.rs` changes only the location of the environment-prefix constant. Signal handling, child reaping, relay behavior, readiness, OOM adjustment, environment decoding/removal/restoration, exec, and status 126/127 are shared unchanged.
- `pinfold-init/src/main.rs:7-33` keeps UTF-8 argument refusal, version behavior, and init help detection before `--`. Runtime entrypoints are still `init [socket]` and `init exec -- argv` (`runtime/mod.rs:419-424,564-575`). Guest command `--help` after the separator remains the command's argument.
- `build.rs:28-37` watches the shared source, new init crate, workspace manifest/lock and toolchain. `build.rs:42-67` selects only pinfold-init into a private target directory. The new package has no pinfold build script, so this removes the host CLI dependency graph and recursion path from the nested build. Workspace membership already uses `crates/*`.
- The spec at `docs/ARCHITECTURE.md:603-607,995-998` names the new packaging and shared source without strengthening guarantees. Linux continues to mount its own full executable. Mac mounts the minimal init executable; host CLI operations inside that helper are not part of the stated guest-init interface.
- Release workflow still builds/stages only pinfold; the additional workspace package does not create a release asset or change released CLI selection.

Test-audit: `crates/e2e/tests/e2e/update.rs:350-353` adds only the actual production init crate to an existing isolated source-copy fixture. It changes no assertions, seam, test function, expected value or sabotage claim. Guarantees 28 and 31 still use distinct real production builds and outside fixture/source/filesystem expectations. The copy is necessary for the existing Mac build path to compile the shared module; it is not a fixture implementation of production init. Lean already for this four-line fixture delta. Existing assertion blocks were read for context; no unrelated suite sweep is claimed.

## O3: Apple running observation reuse (`98c1b7f`)

- `runtime/apple.rs:218-232` returns the actual BoxInfo from the existing native inventory that reports running. It does not synthesize labels or identity from the plan and does not skip waiting without egress.
- `core/box.rs:586-613` still finishes the root socket chmod before readiness reaches the caller, propagates either observation/chmod failure into failed-start teardown, and drains attached output. Podman retains its separate post-ready inventory.
- `core/box.rs:287-291` still compares the observed image ID to the pre-start resolved identity before returning a box. For a fixed runtime generation, the reused labels and image identity are the same creation metadata previously fetched a second time; chmod does not alter them.
- Limit: the returned metadata observation now precedes the transient root exec. This does not establish atomicity against an independent host actor deleting/replacing a runtime box by name. The old later listing also did not establish atomicity, and normal Pinfold peers remain excluded by the unchanged lifetime claim. No stronger guarantee should be inferred.

## C2: resolver in claimed state (`a7495da`)

- `runtime/podman.rs:61-71` writes the same empty file and keeps the same read-only `/etc/resolv.conf` bind. It removes the shared cache-root directory creation and shared resolver artifact.
- `core/box.rs:565` passes the exact prepared claim state directory. Preparation occurs before startup (`core/box.rs:263`); the name lock precedes preparation. `ownership.rs:211-243` records that directory, and `ownership.rs:281-306,150-163` removes its contents only after successful runtime removal while checking the recorded generation. Failed removal retains recoverable state. Failed startup before spawning also cleans the claim. Resolver lifetime therefore matches its mounted box.
- Different normal claims use different state directories. Reusing a name cannot truncate a live claim's resolver because the lifetime lock is unchanged. Cross-XDG callers still coordinate through the same lock. The new resolver has no secret content, and does not expose the enclosing claim directory to a box.
- No generic host-authority or continuous alias guarantee is added. The existing observed-layout admission scope is unchanged; this is an artifact placement correction.

## Remaining validation

Main owns compilation/lint gates and actual Mac/Linux checks. In particular, use actual guest exec/environment and readiness-label/image parity gates for the shared init and O3; verify per-box empty/read-only resolver behavior and state teardown on Linux. Source review alone cannot establish Cargo/Zig incremental rebuild behavior or kernel/runtime behavior. No timing improvement is asserted here.
