# Pin, guest-kernel and CI procedure, 2026-10-26

The pre-release spec pass moved these paragraphs out of ARCHITECTURE.md.
They describe procedure, not the box contract. The spec keeps the rule:
pins move only with a release, every download matches its digest, and a
failure aborts before any pin file is written. This is the text as it
stood at 9f1b440. `scripts/bump-pins.py`, `guest-kernel/` and the
workflows are the current source.

## Nightly pins

Nightly CI repins harnesses, image tools, Rust, Cargo dependencies, prototype
packages and Yard tools with
`scripts/bump-pins.py`. Cargo requirements advance to the latest stable,
non-yanked versions, including incompatible updates; Cargo and Bun regenerate
their lockfiles. Rust advances to the stable channel's version once its Yard
image provides Linux amd64 and arm64; the toolchain and Yard image move
together. CI reads Bun's version from the candidate's image recipe after checkout.
GitHub Actions retain full commit pins and change only in reviewed manual
updates. The Public Suffix List advances to a newer dated snapshot from
`publicsuffix.org`. A version pin only moves up, to a stable version at or
below upstream's latest stable release, and is never
rehashed at its version. pi, claude and codex take the latest release once
its required assets exist. bun, rtk, ponytail and AnyDoc take the newest
version whose release and every required asset (a replaced asset restarts
the count), or on npm every required package version, was published seven
full days ago and is not deprecated. AnyDoc's version must name its native
packages at that version. Prototype packages and TruffleHog
also wait seven full days. Prototype manifests and lockfiles advance in
the same candidate. A missing or invalid publication time makes a
version ineligible. Every artifact download must match its publisher's
digest; a source or integrity failure aborts before any source pin file is written.

Changed pins receive a patch release after Linux x64 tests, the full
Mac suite including live login, and both release builds pass for the
same commit. The prototype typecheck and guarantee 35 run on both
platforms. Unreleased implementation changes since the latest release
block it. A concurrent change to main aborts promotion.
Automatic updates are disabled unless `PINFOLD_AUTO_RELEASES` is `true` and
a dedicated Mac runner has the `pinfold-nightly` label.
Manual runner checks can install missing build tools and run the Mac suite
without updating pins or releasing.
The next suite builds its default image from the candidate's embedded
profile.

## Firecracker and the guest kernel

On Linux, `crates/pinfold/vmm.toml` pins Firecracker and its guest kernel
for `linux-x64`. Firecracker is its GitHub release binary, checked
against its published digest. The guest kernel is pinfold's own build: a
pinned kernel.org 6.18 stable release, verified against kernel.org's
signature, built with Firecracker's x86_64 6.18 guest config for the pinned
Firecracker version, without vsock loopback and with its config at
`/proc/config.gz`. `guest-kernel/` pins and builds it, and its workflow
publishes it as the asset of a prerelease tagged
`guest-kernel-<kernel>-firecracker-<version>`, never the latest release;
`vmm.toml` pins that asset's digest, and holds the one copy of
Firecracker's version, which `guest-kernel/` reads. Both install like a
harness, into `~/.cache/pinfold/artifacts/vmm/<version>/<os-arch>/`, where
`<version>` names Firecracker's and the kernel's, on the first `up`. They
move only with a pinfold release; nightly CI repins Firecracker like a
harness, once a release has been out seven full days, and the kernel's
config and release move with it. The repin waits for the matching kernel
release, which the guest-kernel workflow publishes; it never publishes one.

## Durable prototype launch

Development uses TypeScript and `@types/node` for Bun's compatible APIs;
execution requires Bun 1.4.3+. Launch from the controller directory,
outside the project. Bun loads no `.env` files and auto-installs no
dependencies under its supplied configuration. Exact versions live in
`examples/pi-durable/package.json` and `bun.lock` and advance with nightly
pins, including incompatible versions. Any prototype or dependency change
requires the typecheck and guarantee 35's standalone gate. The gate is
validated on macOS and Linux x64.

(Now in `examples/pi-durable/README.md`.)

## CI and the slow gates

Guarantee 36 uses private network and mount namespaces: `api.github.com`
resolves to a fixture address assigned only there, with a fixture CA
trusted only by the guest. The ordinary Rust suite retains its five-minute
CI budget, measured over the complete `cargo test -p e2e --locked` command,
including compilation. CI fails when that command takes 300 seconds or more.
The two slow gates run separately.

Linux (Firecracker) runs in CI on every push to main and every pull
request, and on a dispatched ref, on GitHub's `ubuntu-26.04` runner,
which grants `/dev/kvm` through a udev rule and loads pinfold's AppArmor
profile for the built executable, keeping AppArmor's restriction on. Before
a push, the operator runs macOS (Apple `container`) on the host, or nightly
CI runs it on the dedicated Mac runner. `scripts/e2e-gate.sh` runs both
suites for the current ref.
