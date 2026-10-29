# Native kernel accounting for G23 and cleanup

Date: 2026-10-29. The approved bounded experiment ran G23 and G15 once
each on a skill-owned disposable Debian Linux VM. One normal integration
test process selected the two exact existing test names, retaining their
exclusive guards and the shared per-cache default-image setup. No binary
instrumentation, production flag, dependency, assertion, limit or source
change was used. This was not a full-suite or CI-budget run.

Both tests passed in **624.61 s**; the external process wait measured
**624.629 s**, exit 0. The source was
`8f7cfd80f4a5cf55a8dc6dc1c4be10f1d9b8db09`, whose production source is
unchanged from `3c9f9d6`. Matching before/after source manifests and
executable hashes were preserved. The static musl executable SHA-256 was
`a9e7604cb7d7122d080c6e8f76e9e1281e009dc4bcccd726ccad6fb50c78ce6e`;
the integration-test executable was
`d0daac4d6ec0c277a0425e7628ffeb9c217c67241089f7f97a288fca502b7978`.
Both measured executables are included in the raw evidence bundle.

## Observer and scope

Native tracefs `sched_process_exec`, `sched_process_fork` and
`sched_process_exit` events used the kernel's monotonic clock in a fresh,
separate tracing instance. The observer used Python's standard library;
no observer package, ptrace or strace was used. G23's existing EIO fixture
uses seccomp user notifications. It remained part of the normal scenario.

Fork ancestry assigns each Firecracker execution to its originating CLI
and named test thread, including worker threads and the EIO supervisor.
Ordinary box VMs are counted separately. Each VMM interval ends at the
last exit in that VMM's task subtree, rather than only its leader's exit.
Phase names follow the existing sequential discovery/build/conversion
order within each build CLI. All intervals were ordered, all relevant
task exits were captured, and neither CPU reported dropped events,
overrun or commit overrun. Parsing found no unrecognized process event.
One event remained unread when tracing stopped; all measured
process subtrees had already exited.

These are process lifetimes, including boot, guest work, host output
handling and teardown. They are not guest CPU times or isolated boot
costs. Kernel event collection also has overhead; this experiment does
not quantify it or establish a causal speed comparison with CI. No
guest-writable filesystem was mounted or inspected by the observer.

The first observer attempt rejected ambiguous Cargo artifact metadata
before tracing or either fixture started. Selecting the integration-test
artifact by target kind corrected that local driver error. The actual
two-test invocation was not retried. Acquisition's first permission review
timed out before execution; its permitted retry acquired the lease.

## Observed build transactions

| Work | CLI calls | Discovery VMs | Build VMs | Converter VMs | Build CLI total |
|---|---:|---:|---:|---:|---:|
| Shared cold default setup | 1 | 1 | 1 | 1 | 75.472 s |
| G23 case builds | 13 | 13 | 13 | 11 | 262.036 s |
| G15 case builds | 11 | 11 | 11 | 10 | 241.770 s |
| Total | 25 | 25 | 25 | 22 | 579.277 s |

There were **72 build-role VMs** and **12 ordinary box VMs**. G23's
intentional RUN failure and successful identical repeat each ran two
build-role VMs; G15's successful identical twin also ran two. The passing
outside assertions distinguish a failed build from completed-root reuse.
All other successful build calls ran three VMs. This replaces the static
conversion bounds for this invocation only; it does not supply the
unobserved conversion counts in the previous full CI run.

The first-to-last observed CLI span was 364.514 s for G23, including cold
setup, and 259.827 s for G15. These spans exclude small portions of the
fixture with no CLI process alive. Test-thread wall times are not summed:
libtest starts both threads and one waits for the exclusive guard.

## Measured phase and host intervals

| Work | Before first VMM | Discovery lifetimes | Host gaps between VMMs | Build lifetimes | Converter lifetimes | Last VMM exit to CLI exit |
|---|---:|---:|---:|---:|---:|---:|
| Shared cold setup | 23.707 s | 4.417 s | 0.582 s | 28.552 s | 16.531 s | 1.681 s |
| G23 case builds | 75.344 s | 44.354 s | 8.236 s | 60.046 s | 66.551 s | 7.505 s |
| G15 case builds | 63.151 s | 32.244 s | 7.474 s | 43.508 s | 85.457 s | 9.936 s |

Each row partitions its build CLI total, subject to rounding. The final
column includes verification, publication and cleanup; process events
alone cannot split those operations. About 45.35 s of the whole invocation
lies outside build CLI intervals, including ordinary box operations and
test/runtime setup and cleanup.

Here warm means the common artifacts and default setup already exist;
the fixtures still execute their specified no-cache builds. Warm
preparation before the first VMM took **5.723–6.277 s per call**,
median 5.737 s; its total was **138.495 s**. The first init-disk mkfs
process took **0.727–0.832 s** per warm call, 18.322 s altogether. The
preceding in-process interval took 4.798–5.257 s. Source preparation
verifies the pinned cached 1 GiB builder root and copies the init binary
before that mkfs. The observer does not split hashing, copying and other
in-process work. It therefore does not prove a saving from skipping any
operation or justify weakening artifact verification.

## Decision supported by the measurement

The warm case-build VMM lifetimes alone total **332.160 s**. Including
cold setup gives **381.661 s**. All host intervals inside build CLI calls
total 197.616 s. Even removing all of those host intervals would leave
about 427.01 s of this invocation, including the ordinary box work and
other test activity. Thus init-disk reuse or other preparation-only work
cannot meet 300 s for even these two fixtures on this VM. The disposable
host is not the CI runner, so this is not a proof of a universal lower
bound or an exact CI saving.

The observation does not isolate boot from guest work and does not prove
that merging roles would meet the budget. No bounded production
optimization with an evidenced path to 300 s follows from it.

The architectural decision for Adam is whether to authorize a design
review of the build execution lifecycle to amortize repeated role
initialization across builds, or retain the current finite-role design
with runtime admission blocked. Such a review must account for the
separate discovery/build/conversion privileges and disks, fresh ownership,
immutable inputs, proxy enforcement, cancellation, finite deadlines,
leases, bounded output validation and atomic publication. Reusing a
running role across build requests changes the lifecycle and reset/trust
model; it is not an implementation shortcut. No shared guest, snapshot
reuse, merged role, weaker check or budget waiver is approved here.

No further fixture cut, performance probe or full-suite run followed.
Numeric-policy approval, runtime merge and CLI release remain deferred.

## Preserved receipts

Evidence root:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| File | SHA-256 |
|---|---|
| `kernel-observer-source-8f7cfd8.tar.gz` | `dc70fcc6eda42e098944e70f58baaec6066793de7df36d3aeab5c4df774cc578` |
| `kernel-phase-g23-g15-8f7cfd8.tar.gz` | `81cb626f1d2eab1026902276bd6ede5aa7a6e4d57a6d7e6b0345695f8c139495` |
| `pinfold-kernel-phase-observer.py` | `c14f5474fa2163d4428cea879217924dc814796bdf72130dcc7e8c0b10a5e480` |
| `kernel-phase-analyze.py` | `6168127685666cd975f7e637083e58892c4e5e95468dcdce86e3670613ec0d81` |
| `kernel-phase-evidence/trace.raw` | `f60d3d07878f979305ad5273859dab1b5835af0f171ae5cbd8f1d733d6a2c113` |
| `kernel-phase-evidence/analysis.json` | `368fd77027f5fd922b52f7ea17b6ee2449a60863d8cd2847b0b8a702b739a235` |

The raw bundle was copied out and opened successfully before releasing
the VM. `labvm release` returned `ok: true`, `status: idle`. Analysis and
this note were produced locally after release.
