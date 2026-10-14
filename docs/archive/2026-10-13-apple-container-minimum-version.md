# Apple container 1.5.0 minimum

Apple box startup now requires stable `container` 1.5.0 or newer. This is
the version named in `2026-10-10-test-audit.md`: the recorded full Mac suite
passed 33 tests, with zero failures and one ignored test. It is also the
installed host version verified during this change.

The preflight retains its existing single `container --version` process.
Apple and Podman share the numeric stable-version comparison. Doctor now
runs preflight on both platforms and skips runtime-dependent probes when
it fails. Listing and cleanup remain available on older runtimes. No
runtime is installed or upgraded by Pinfold.

## Validation

- `cargo fmt --check` and all-target Clippy with warnings denied passed.
- The existing `cli::doctor_reports_without_changing_state` end-to-end
  test passed against the installed Apple runtime.
- The official signed 1.4.1 package was downloaded, verified against its
  GitHub release asset SHA-256, and extracted into a temporary directory.
  It was not installed. Its real CLI on a private PATH caused `box up`
  to exit 1 with `event: refused`, `reason: runtime`, and no box state.
  Doctor displayed the installed version and minimum-version failure.
- Installed 1.5.0 passed doctor preflight and a real box startup, exec,
  and stdin-triggered teardown using cached Debian trixie-slim. The only
  native container before and after was the existing `buildkit` service.
  No image build or runtime service restart occurred.

The smoke probe used private Pinfold directories and a current maintenance
stamp to exclude unrelated retention. Its temporary state was removed.
No permanent test, runtime stub or test-only production path was added.
The full Mac suite was not rerun for this version-only admission change.

## Evidence

Local raw results, doctor output and probe script:
`/private/tmp/pinfold-apple-floor-20261013/`.

| Artifact | SHA-256 |
|---|---|
| Pinfold Mac validation binary | `2d235a7ad8211532d3b3671414f87a95f5c5b9e1b6eb3e4014e1c447a90d116d` |
| Official signed container 1.4.1 package | `c0d2716afefbb194c93fae662e9cae7cc186bcbcf746816608ec673dd648a6a4` |
| Extracted container 1.4.1 CLI | `50edb17e6f1016c135cb9eed6f60cf9302d0091b39d67e155e952201848ac9c4` |
