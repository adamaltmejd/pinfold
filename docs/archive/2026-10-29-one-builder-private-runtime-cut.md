# One-builder private runtime cut, 2026-10-29

This private cut joins the reviewed Rust runtime, binding spec and actual
producer artifact pins. It remains unpushed. The candidate's intended public
artifact URL is not published. Normal-CLI proof must use the existing verified
offline archive path. Separate approval is required before publishing or
adopting public artifact bytes. Existing public bootstrap 093 remains untouched.

## Source acceptance and producer evidence

Both independent operator source reviews accepted bounded native proof.
The reviewed runtime code remains unchanged. Host handoff sizing uses logical
file lengths and fixed directory charges. Allocated-block sizing belongs to
the separate converter and does not determine the handoff reservation.
The source-freeze note records the earlier uncommitted candidate and its
then-uncompiled helper. This note records the later exact producer result.

The operator built producer commit
`ad332ed60377f5f9de01884d78f94f94a1e878cc` through the pinned factory and reports
114.819 seconds for that factory build. This is producer duration, not image
build latency. Its 12 retained assets were independently checked by the
operator, including checksums, full gzip, single raw-root content and source
identity. A further artifact audit was still running when this cut was prepared.
The worker checked the retained manifest and helper-build sidecar hashes against
the independent verification receipt, and matched every producer source input
against the manifest. No producer bytes were rebuilt or guessed for this cut.

Evidence is retained by the operator under
`/Users/adam/.codex/worktrees/ed8e/pinfold/.local-evidence/one-builder-latency-2026-10-29/`:
`manifest.json`, `discovery-build.json`,
`producer-independent-verification.json` and `producer-assets.tar`.
The manifest SHA-256 is
`4dd78a2b547d99432c9878a4d5be24147646bf9fd62cebccdaaca108345929f3`.
The helper-build sidecar SHA-256 is
`2c1a81fff91d43fe2d1482b7150bb57aaf5079c8f183ad681bf603f7faf904b3`.

| Built object | Bytes | SHA-256 |
|---|---:|---|
| Archive | 122592271 | fa46484cbcf7ce04eabecd31f6a6fc06dc6a7389390276aadc3f22866c731ffb |
| Raw root | 1073741824 | a245c096e2025502c5fe15eeb8fae391503c440edaa53e0d85e4042ec62aeb01 |
| Compiled helper | 22294690 | d038bd1623499d5ef47b1aec9659854b72eed48ae2f7d066ee6f0c0a358cc738 |

The measured recipe SHA-256 is
`7e5e5778257eb26d5b768801d0b68b6f151cae8a4eef11df692f411f7b39d049`.
The compiled helper source SHA-256 is
`28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe`.
BuildKit, Go, package and upstream source pins are unchanged.

The intended URL is
`https://github.com/adamaltmejd/pinfold/releases/download/builder-root-fa46484cbcf7ce04eabecd31f6a6fc06dc6a7389390276aadc3f22866c731ffb/pinfold-builder-linux-x64-fa46484cbcf7ce04eabecd31f6a6fc06dc6a7389390276aadc3f22866c731ffb.tar.gz`.
This URL is not published. The pin identifies retained factory bytes for
authorized private proof; it does not establish public installation.

## Consumer change and remaining proof

One finite VM now performs native metadata discovery and the build. The host
keeps unconsulted local configs private, records each served consultation and
permanently closes metadata lookup before supplying selected inputs. Only
aliases and opaque identities are provided upfront. The host formats separate
private staging, then copies its bounded bytes through the retained fixed-length
backing FD. It syncs and closes writers before continuing. The guest invalidates
the quiescent slot and mounts it read-only. Conversion stays a fresh independent
VM. Final cache-clean, audit, cancellation, output admission, leases and atomic
publication barriers remain in place.

The five reviewed helper/runtime files retain their frozen SHA-256 values.
Only the spec's archive acquisition paragraph additionally changes to describe
the actual private pin, unpublished URL and 122592271-byte bound. README files
now state the same status and current finite-builder mechanism. No test, caller
schema, public flag, dependency, resource cap or new fallback path is added.

The helper is compiled by the exact factory result above. The changed Linux
Rust consumer remains uncompiled at this snapshot: the earlier Mac cross-check
stopped in ring's build script on missing x86_64-linux-musl-gcc. This cut runs
only cargo fmt --check and git diff --check; it does not repeat that unavailable
cross-check. The operator owns native Linux compilation and proof.

No consumer VM, suite or image-build measurement was run by the worker. Source
acceptance and producer compilation do not prove handoff coherence, lifecycle
behavior, runtime controls, image-build latency or the suite budget. The bounded
proof plan remains in the source-freeze note. No artifact publication or public
push occurred.
