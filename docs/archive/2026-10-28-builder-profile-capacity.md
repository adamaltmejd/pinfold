# Built-in profile capacity observations

Date: 2026-10-28. Read-only audit of retained normal CLI runs, not a new
execution. No test, build, VM or Git operation was run for this audit.
Numeric resource policy remains unapproved.

Evidence directory:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| Retained file | Bytes | SHA-256 |
|---|---:|---|
| `current-builtin-profiles.tar.gz` | 556342670 | `c780de62a84cd1449e95891315e897c0ff845f52e758d141f3fc0de85c410d89` |
| `linux-profiles-current-1.log` | 3585 | `e9a15f61a92423064e156143fae406619b82df3fce3f37c9c9d11c74d190aca0` |
| `linux-admission-hup-compile-1.log` | 1981 | `a38d007fccf038de8caccd2f7e0bf02145e8fea483306c09109094c1dd3c9c84` |

The bundle's `current-profiles-proof.py` is 2,418 bytes, SHA-256
`94664ca9e7b130f710471043b2cd874354029c803ccb56a9beaa337ef3c70203`.
It runs the normal CLI sequentially, captures stdout/stderr and elapsed
time, reads the resulting catalog and records host child-resource counters.
The profile driver ran after `linux-admission-hup-compile-1.log`, which
records the CLI path with SHA-256
`641a3a44fba3449e3062d060d4b62d1088110299a9af9d3c087780fae17c2b85`.
That rebuild includes the build-entry preflight guard and Linux runtime
refusal mapping. The retained local compile-log timestamp is
2026-10-28 22:34:59 +0100; the profile log is 22:40:24 +0100. Its missing-KVM
result is `event: refused`, `reason: runtime`, consistent with that mapping.
The older `linux-preflight-durability-compile-1.log` records binary
`9da0f4ba3915b10534fcf6b5f131a9af486c6f9920fbfa57658600db668d37b8`,
whose earlier missing-KVM trial returned `event: failed`; it is not the
compile receipt associated with these profile observations. The driver
does not independently hash the executable at invocation, so this remains
a compile-order and behavior association, not a direct invocation-time hash.

## Successful current profiles

Both `pinfold build --profile documents` and `pinfold build --profile full`
exit zero and print their normal `localhost/pinfold/profile-<name>:latest`
reference. The retained catalog maps each reference and its unique build
tag to the corresponding identity below. The current built-in fragment
concatenations match their recorded Containerfile fingerprints. The driver
does not launch either completed profile in a box.

| Profile | Wall seconds | Completed root bytes | MiB | Bytes below 512 MiB | MiB below cap |
|---|---:|---:|---:|---:|---:|
| documents | 88.231067 | 392686592 | 374.495 | 144184320 | 137.505 |
| full | 94.037217 | 451499008 | 430.583 | 85371904 | 81.417 |

The two archived roots were streamed and hashed as opaque bytes. Their
lengths and hashes match the catalog; neither filesystem was mounted,
extracted or inspected. Cap headroom is arithmetic on emitted ext4 lengths,
not free space measured inside either filesystem.

| Profile identity | SHA-256 |
|---|---|
| documents runnable manifest | `2d002f77fe16c8eb9194c3bdcf9e6cbc2bbf58f0dd08f35d13b165da38747098` |
| documents completed root | `0d595360127897b9108ead3af4729428f2799565fc176af7a8fb75f5e79be7e4` |
| documents Containerfile | `0b085a8b4741ff84755b67751c2c26539776d427e99a06a81d1b0fe93d8d9943` |
| full runnable manifest | `7aa20bd4f6010b8611b3e582f75f25437a52f20bcc9b1e91cb924c279b9f9d3f` |
| full completed root | `ab3f06c05ed12378f09b626ab84d0bf371ac29e257ebd968422c38a58be5d53e` |
| full Containerfile | `2b6457e3e314d8cdb6c45ac675ba834e717a368713f89d9ed238b5e14ea3fe15` |

Both catalog entries record conversion recipe
`6315dddb905160ec5ea9a2003813f25656760a1fe8e28a0faead3b21eb5d55d3`
and external Debian source manifest
`sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4`.
Their launch records name bootstrap archive
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.
The documents run is
`05ba13387c0fefbbb3ce28f708e2b7ce884255c0f969a539f3f5774562302f99`;
the full run is
`aa878741504bfe87767392aef7b1e54ec6415f37b1cbc4bf655577e777109ca1`.

## Resource definitions and limits

| After profile | Child CPU seconds for this call | Cumulative child max RSS, KiB | Builder directory allocated bytes | Images directory allocated bytes |
|---|---:|---:|---:|---:|
| documents | 105.816888 | 1974940 | 1034002432 | 790380544 |
| full | 111.377464 | 2179912 | 1807634432 | 1393188864 |

CPU is the before/after difference of `RUSAGE_CHILDREN.ru_utime + ru_stime`
around each CLI call. It includes waited descendants accounted by Linux,
not just CLI CPU. RSS is the driver's cumulative `ru_maxrss`, without a
before/after subtraction: the largest waited child's resident-set maximum,
not a simultaneous process-tree total or a per-build/guest-phase peak.
The second observation can include earlier children. These definitions follow
the [Linux getrusage interface](https://man7.org/linux/man-pages/man2/getrusage.2.html).

Disk figures are `du -B1 -s` after each completed CLI command, covering the
whole existing `/home/agent/.cache/pinfold/builder` and `images` namespaces,
including directory allocation. The cache was already populated by earlier
trials. They are allocated host blocks, not sparse apparent lengths,
per-build costs or peak disk use. The bundle's catalog also contains earlier
images whose bytes are not all included in this profile bundle.

The launch records label policy `candidate`: 2 vCPUs, 2,560 MiB guest memory,
8 GiB builder backing disk, 3 GiB converter work disk, 512 MiB input and
300 seconds per job. The source also provisionally caps emitted roots and
receipt payloads at 512 MiB. These two successful profiles do not measure
converter-work peak, backing-filesystem free space, peak host allocation,
aggregate input size, output-receipt headroom, memory peaks by phase or a
capacity/refusal matrix. They do not establish cold-build timing, future
profile growth, concurrency capacity or compliance with the suite budget.
No numeric limit is approved by this observation.

## Missing KVM

The same driver runs `pinfold image build no-kvm-current` through `setpriv`
as uid/gid 1000 with supplementary groups cleared and environment reset.
It exits 1 in 0.016009 seconds, emits `event: refused`, `reason: runtime`
and a `kvm:` permission-denied detail, with empty stderr. The driver asserts
that result and byte-for-byte equality of the catalog before and after.
`current-profiles-proof/kvm-catalog.json` records both hashes as
`c59d5189439b00611cc6094f4fbe5e7fa31dbea1cc995260d4b2db56a0322613`.
This establishes runtime refusal and unchanged catalog, not absence of all
CLI maintenance or filesystem writes elsewhere.
