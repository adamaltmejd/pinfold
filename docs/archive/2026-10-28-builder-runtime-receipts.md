# Builder runtime receipts

Date: 2026-10-28. Source baseline:
`b2071840ee5b1aca12720a3600a817278f586ac2`, with the subsequent image-job
preflight call and pending row 23 test edits. Evidence is retained locally at
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
The observations below come from those logs and named receipt files, not a
new execution during this documentation pass.

## Executed identities

The initial integrated smoke log records Linux musl binary SHA-256
`f1fa2be146850ea1832ed4ebe62d830f410ff2552192a78f8a935e5d9594a92a`.
The preflight/durability rebuild log records
`9da0f4ba3915b10534fcf6b5f131a9af486c6f9920fbfa57658600db668d37b8`.
These are hashes recorded by the trial, not rehashed binary bytes here.
The retained source delta contains `job.rs` SHA-256
`194be05f7ec6e6b42df8c27d787a5e15440f4e2a5ad08330a1013068662163f1`;
its bytes match `runtime-delta-source-sha256.txt`.

The pinned helper bootstrap is a 122,578,909-byte archive, SHA-256
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.
The local archive hash was recomputed for this note. Its producer manifest
and independent receipt agree on the 1,073,741,824-byte raw root SHA-256
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`
and recipe SHA-256
`d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.
They record `/usr/local/bin/pinfold-discover` as 22,282,402 bytes, SHA-256
`0b8927187764740c328a245e21c44a08cb419aa93911d3be624e4c943f2907d0`,
built from helper source
`bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335`.
The recorded producer uses Go 1.26.10 and BuildKit commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e` with its vendored dependencies.
No public bootstrap publication is established by these local receipts.

## Normal CLI observations

- **Integrated smoke and reuse.** `linux-integrated-helper-smoke-3.log`
  records exit 0 and `built` for image
  `sha256:33386dddcacd1c4d255e285cc0bb2b4211b90340dd276e9f7a3ba2e55c25cebe`.
  Its retained catalog maps unique ref and latest to that image.
  `ready-reuse-proof.tar.gz` records successful public `clean`, a subsequent
  launch with the bootstrap absent, ready on that image, and real `box exec`
  returning `integrated-helper-root\n`. Its final down event is `signal`.
- **Local aliases and config-only external base.** The partial local-zero
  bundle records two names sharing image identity, then an ARG-selected local
  base plus COPY from the other alias. The resulting box reads the expected
  `integrated-helper-root\n`. A separate build starts from immutable
  `tonistiigi/test:nolayers@sha256:acf1bec0759497a99077fe034707bb64ef675aee61e9902e26af53aeb1c22e0c`;
  its recipe checks `foo=bar`, and its real launched shell returns `bar\n`.
- **ONBUILD.** The same bundle's parent recipe stores `ONBUILD RUN false`.
  The parent builds successfully. Its child FROM that local parent fails
  the actual inherited `RUN false`. This distinguishes deferred conversion
  from deleting the stored trigger.
- **Missing KVM.** The continuation invokes the normal CLI as UID/GID 1000
  with supplementary groups cleared. It exits 1 with `kvm` in the failed
  result. The retained catalog hashes before and after are both
  `a75b11d9363a39f7a57f61a1857c841a5922392c8150a053f745a26babe43521`.
- **Post-commit directory-sync EIO.** The continuation's external kernel
  supervisor records `injections=1`, `injected_errno=5`,
  `supervisor_errno=0`, and child exit 0. The selected directory is device
  2049/inode 531275; catalog inode changes from 531295 to 531314 before the
  injection. The normal CLI reports `built`, and a box launched from its
  emitted unique ref reads `integrated-helper-root\n` from
  `/durability-marker`. This demonstrates successful visible publication
  despite that injected post-commit sync error.

The first local-zero driver ends with an assertion failure after its per-case
results. It is retained as a partial bundle, not a successful aggregate run.
The separate continuation has its own successful receipt and catalog-hash
comparison. Earlier smoke attempts also failed before the retained successful
smoke; this note does not count those attempts as passes.

## Suite boundary

`mac-runtime-current-1.log` reports **24 passed, 0 failed, 1 ignored** in
171.46 seconds. Its exit-zero wrapper receipt records 172.92 wall seconds,
baseline `b2071840ee5b1aca12720a3600a817278f586ac2`, and dirty test,
preflight and documentation files. This is a pass of that recorded source
snapshot, not an assertion about later edits.

The retained Linux `linux-row23-current-2.log` has since completed with
**0 passed, 1 failed** in 309.25 seconds. Its receipt assertion sees one EIO
injection and child exit 0, but `supervisor_errno=5` instead of zero. It is
not a row 23 pass or a Linux suite/budget pass. No cause is assigned here.

There is no acknowledged cancellation-consumption proof. The EIO case does
not establish cancellation ordering or physical crash durability. There is
also no retained ACL component proof against this current helper bootstrap
root. The normal CLI cases above do not establish general ACL fidelity or
complete all runtime/security release gates.

## Verified retained files

SHA-256 values below were recomputed from the local files. Receipt JSON was
read directly from the owned evidence bundles; no image layer or root
filesystem was extracted or mounted.

| File | SHA-256 |
|---|---|
| `ready-reuse-proof.tar.gz` | `f7c5a9ecc3c8c903b7f358621da9d4c18eb973717818d10fb9cb16f280f26908` |
| `integrated-smoke-image.tar.gz` | `21c3ab46fd8efc531ed1a70c3d8ac0e3a024c0783e0b296c2b10365772b039a6` |
| `local-zero-proof-partial.tar.gz` | `a89f5a15f54b8192d713d2ca65e46a3715b06db3c0abd9e11cd352fd565c15cb` |
| `durability-continuation.tar.gz` | `77695e60a22947f8f7c158415e304912fe05297186a2b552a6aca1b8391dc4ef` |
| `runtime-delta-tested-source.tar.gz` | `74dea8067c05c415a04ab7be44c2b9ea5d2e80010c58f4100c649b9637b21107` |
| `linux-integrated-helper-smoke-3.log` | `d2f050ae2a691f593e2c6a8608bc7e717d4fbdfd07a76baf2ebd599f88877eea` |
| `linux-preflight-durability-compile-1.log` | `2ef47a72e27b42332c3543baa5f5296d3353e321adb4a27ac944ed81279ad33c` |
| `mac-runtime-current-1.log` | `53eb6a95fd29d5d1979fa11378cfb85758929c034d071d4fd44c753ba7151318` |
| `mac-runtime-current-1.json` | `2f11226b003a210b4f7ea36d049ec6d365e4b2a18f38e25e288134c2c71230bd` |
| `linux-row23-current-2.log` | `a48b3696a2e4fcce4e97d5fa43b75d5be08275d69994924367a5b4981e53ac39` |
