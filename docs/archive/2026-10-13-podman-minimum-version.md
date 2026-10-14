# Podman 6.1.4 minimum

Pinfold's Linux box preflight now requires a stable Podman release at least
6.1.4. The check uses the existing `podman info` response and adds no process
or dependency. Older and prerelease versions refuse startup with the
existing `runtime` reason. Doctor reports the failure. Inventory, cleanup,
and image commands retain their existing behavior. Apple is unchanged.

The requested support floor is not evidence that issue #89 is resolved.

## CI provisioning

The published Ubuntu 26.04 runner inventory lists Podman 5.7.0. The shared
Linux setup script therefore builds unchanged upstream 6.1.4, verifying
commit `464c15fec06f0074c916f9eff87c4806869c2833`, with its default native
build tags. Linux release assets are remote clients, not this runtime.
It installs the binary and that release's stock storage configuration.
Distribution runtime helpers remain in use.

The matching storage configuration matters: the Debian 13 vendor template
explicitly set root-owned graph/run paths. With it, 6.1.4 failed as a
rootless user while creating `/var/lib/containers/storage/libpod`.
The upstream template leaves those paths unset, allowing rootless defaults.
Installing that unchanged template restored ordinary rootless operation.

## Validation

A disposable Debian 13 x86_64 VM exercised the real Linux Pinfold binary:

- Distribution Podman 5.4.2: `box up` exited 1 with a `refused` event and
  `reason: runtime`, reporting the minimum. Native inventory remained empty.
- Unmodified Podman 6.1.4: doctor, box startup, exec, filtered listing,
  stdin-triggered teardown, and empty final Pinfold/native inventories passed.
- The box used Alpine 3.23, image ID
  `f6d31b983274a75183386cda3aac54849498fac0fc3e0d98f92ff56ed517d92b`.

The initial smoke script omitted the required listing label filter. Its
startup and exec succeeded; its listing invocation was invalid. Correcting
the probe to use `--label dev.pinfold.owner` produced the passing lifecycle.

The VM build used the CI provisioning steps without the existing GitHub
runner AppArmor relaxation. No VM security setting was weakened. Yard's
installed runtime and active store were untouched. Evidence was exported
before releasing the disposable VM.

`cargo fmt --check`, native and Linux-target Clippy with `-D warnings`, the
Linux musl build, shellcheck, shfmt, shell syntax and diff checks passed.
Linux cross-compilation used the installed Zig toolchain.

Neither the full contention workload nor the hosted Ubuntu x86_64/arm64
suites ran. This does not establish LXC compatibility, throughput gains,
or that source provisioning fits the hosted jobs' time budget.

## Artifact identities

Source checkout before the change: `198600b`.

| Artifact | SHA-256 |
|---|---|
| Linux Pinfold validation binary | `c450717357c400930f6b904cb4c0a4ea61eb5c6ee48eef2a7c58a508256f768a` |
| Stock Podman 6.1.4 validation binary | `6db4160f7d4393005430d36b96994d886c9d8e956748b450c91ee3104b19f755` |
| Upstream storage configuration | `322dffd5a543bb9e1ca9c67cc56999c62b746118674d004180fe60dddfc21979` |
| Evidence archive | `225f2449ef81a8e0b55880b59298108ce3565a67d5bad1e3939588963986704f` |

Local evidence: `/private/tmp/pinfold-podman-6.1.4-minimum-evidence.tar.gz`.
It contains runtime reports, lifecycle output, build logs, the smoke script,
the stock storage template and artifact checksums.
