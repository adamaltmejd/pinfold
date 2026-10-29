# Current audit candidate: completed validation

Date: 2026-10-29. Execution head:
`454a6a71a18bfa0839a405fd70a8ca9fb9ad8dd5`.
This supplements the earlier audit snapshots; it does not rewrite them.

## Source and review identity

The ordinary merge preserves public PR 250 head d76ea72 without force.
Its complete tree equals 28ef2c4:
`4cfcaa9be7e01f24243ad50c2130600dfe429980`.
Both independent operator reviews passed 54ba78d's source, coverage,
metrics and spec/test audit, then passed 454a6a7's history/Mac-receipt delta.
The operative files remain identical to the clean 54ba78d Mac-suite head.

Linux CI checked out GitHub merge
`ed5792f90ab8c5484c8dfc18caf74d8acc6ef0af`, whose parents are producer
main 5c0ffa2 and audit 454a6a7. Its complete tree is the same hash above.
The manually dispatched G36 workflow checked out 454a6a7 directly.
No producer input, test assertion, control, pin, dependency or version
changed during these gates.

## Results

| Gate | Actual result |
|---|---|
| Full Mac suite, clean 54ba78d | 24 passed, 0 failed, 1 ignored; exit 0; 100.85s tests, 101.816s command; live login passed |
| [Linux x64 CI](https://github.com/adamaltmejd/pinfold/actions/runs/40849946746) | Format, strict Clippy and AppArmor passed; 25 passed, 0 failed, 1 ignored; exit 0; 200.29s tests, 202.936516654s command, below 300s |
| Mac arm64 release build, 454a6a7 | `cargo build -p pinfold --release --locked --target aarch64-apple-darwin`; exit 0; 16.825412989s command |
| Native Linux x64 musl release build, 454a6a7 | `cargo build -p pinfold --release --locked --target x86_64-unknown-linux-musl`; exit 0; 53.85s command |
| G34, Mac, 454a6a7 | Existing ignored test run once with `--ignored --exact`; 1 passed; exit 0; 335.95s tests, 337.319759846s command |
| [G36, Linux CONNECT](https://github.com/adamaltmejd/pinfold/actions/runs/40850103047), 454a6a7 | Passed the real private-namespace proof; twelve outside idle-closure observations, 300.03267325–300.092632706s after the final activity |
| [Guest kernel](https://github.com/adamaltmejd/pinfold/actions/runs/40849946688), 454a6a7 | Build passed; Publish job skipped |

The Mac full-suite command was unfiltered `cargo test -p e2e --locked`.
G34 used `cargo test -p e2e --locked --test e2e
box_::blocked_route_responses_release_the_upstream -- --ignored --exact`.
Both used the normal Mac Cargo runner. G34 retains the production deadline;
it is separate from the normal-suite budget. No unchanged test was retried.

Before G34, the runtime worker was idle and a host process check found no
active suite. The post-run check found none; the Mac slot is released.
The native release build used this audit's disposable Linux VM lease,
with the pinned repository toolchain and musl-gcc as in release.yml.
Its checkout was clean at the exact head before and after the build.
The binary was copied out and verified before `labvm release` returned
`idle`. No VM lease remains owned by this audit.

## Failed build attempt

The preceding Mac-hosted Linux cross-build, `cargo zigbuild -p pinfold
--release --locked --target x86_64-unknown-linux-musl`, failed with exit
101 after 49.017779112s. Zig's linker/cache writes reported `NoSpaceLeft`
and `No space left on device`. This is an infrastructure failure, not a
passed build or product defect. It was not retried unchanged. The distinct
native Linux build above supplied the successful release-build evidence.
No shared-store or global cache cleanup was run by this audit.

The older 3d6b106 Mac run remains failed: 23 passed, 1 failed, 1 ignored,
exit 101, 438.252s. Its separate upstream-429 observation and intentional
stop/137 remain as recorded. No later pass changes that history.

## Retained receipts and binaries

Private directory:
`/Users/adam/Code/pinfold/target/builder-artifact-audit-current-2026-10-29/`.
Logs/JSON are mode 0600; executable copies are mode 0700. The two release
binaries ran on their native platforms and reported `pinfold 0.3.3`.
The Linux release file is an x86-64 static PIE; the Mac file is arm64 Mach-O.
The Mac suite CLI and e2e executable were also copied out unchanged.

`receipt-inventory.json` records all nineteen files below. Its own size is
3130 bytes and SHA-256 is
`f78417c0b0ad635172b9025dd97d5c4e06f0a92e0df1b147757dd5a74c0e1fb7`.

| File | Bytes | SHA-256 |
|---|---:|---|
| e2e-mac-suite | 5425720 | 1fc5c0812b8a2a92a4ac1e7d75dfab3645a9be5eda288ae0931e3d2f0b70a621 |
| g34-mac.json | 374 | 2f194a311bef55db5c77542c671353e1aaada4d4891b2cf94f4222cd0d559631 |
| g34-mac.log | 532 | bfd10418aae2e4f073bde525eb726eb78652111d9cbd7c567e23edcddb43ff13 |
| g36-linux.json | 725 | 74fc162ce8acb378cd5f19ed438357cf65aa91a33913d70a30eda049cc7fb648 |
| g36-linux.log | 84021 | bea3f49bcccac8c47cbae7878d605625b91091c6d99684d4b8956a08b1298877 |
| guest-kernel.log | 32831 | 2cad9e864ef55c02e13a021d653dd71b952c56269b1c01aca5662ebe8187e3ef |
| linux-ci.json | 387 | 0f8897d295b60669d4a3dc2d3580f36e9db68fd432dea37c435dccc5d5a64a6e |
| linux-ci.log | 66732 | 151efd536d277b6bba0ab791e834afb30cbdf924780ec1695a3d7126f0a3628e |
| mac-candidate.json | 342 | 71adfef34f75fa1ee396357ed5d55ebcb05a829f347439a76a8e58455aebb708 |
| mac-candidate.log | 4358 | 5a58de074f45c5d605facc6d6dc29da570659770169975a2852980c99d44c40e |
| pinfold-aarch64-apple-darwin | 6375328 | ff6ec1b13db4397d04dac5f57876e897220aa47cb296aa1e47de2a21b626426f |
| pinfold-mac-suite | 18510504 | a990dce75cb433f4892454a1c201a00c3c5b84f8eefdcebc773c38642d16b45c |
| pinfold-x86_64-unknown-linux-musl | 6168024 | 83eeb56be18cc9d522d1924007efb27a74984c882d78a17337f4507468fc0363 |
| release-linux-cross.json | 340 | d3625a584b2b060c0eceea3568c3c03c51bb34af285ab11048e8bef0d6a62fad |
| release-linux-cross.log | 7043 | c99b4a808feb3f7fc6909301dc7982eca03028b8ccfa1242fc687ccfe4f1262d |
| release-linux-native.json | 488 | a4615727f744dba9dfd35ecc7f668e16f9060750b45d99aec3178b5c3a8681ec |
| release-linux-native.log | 22322 | 43eaa21096f4fa309f1e74850aaf1cc0b0bb8ba0985525bf4fcfc0308396cd3d |
| release-mac.json | 259 | 75d7a417730065bc5635ae7da352b1cc06a22255d27689a3653ddbd05b57d062 |
| release-mac.log | 1976 | 2f98e11263536343a0beac798e21483f43c4f5a5e4a80aafaea3992cd8820548 |

## Remaining decisions

T1/G9 live-stream observation, T2/G2 correlated plain-HTTP allowance and
T3/G39 independent scan coverage remain open in the current written
tickets. All existing assertions remain. These are inherited proof
limitations, not reproduced product bugs or repaired guarantees.
Their artifact-release disposition still needs the operator/Adam decision.

Q1 duplicate-login validation and Q2 Apple shutdown/version support remain
separate control-review questions. No control change or version-floor
change was inferred from the audit or these passing gates.

The operative publication proposal is still the twelve preserved 09393a02
assets, targeting producer main 5c0ffa2, in
`2026-10-29-builder-artifact-readiness.md`. These private CLI build binaries
are validation evidence, not additional proposed release assets.
Runtime PR 252's consumer/resource/fault gates and numeric policy remain
separate. Neither its implementation nor its results are adopted here.

This receipt is a documentation-only follow-up and still needs the
operator's final delta review. Draft PR approval grants no PR merge,
artifact upload, tag, release or consumer admission. Promotion still
requires the explicit ticket/control dispositions, final review and Adam's
separate decision naming the exact twelve assets, tag and producer target.
