# 512 MiB bootstrap producer trial, 2026-10-29

This producer-only trial changes one executable expression in build.py:
the newly created raw bootstrap file is sized to 512 MiB (536870912 bytes)
instead of 1 GiB. It does not resize an existing image. The mkfs command,
package closure, cleanup, metadata handling, BuildKit, compiler, helper,
upstream sources, keys and pins are unchanged. This is read-only bootstrap
geometry, not a build, cache, input or output policy limit.

The operator's prior sizing audit reported a 329.94 MiB closure and a
461.57 MiB resize estimate. Those observations justify trying 512 MiB;
they do not prove that this fresh filesystem fits. The existing source
comment predates this trial. No successful 512 MiB build is claimed here.
The purpose is to measure whether reducing the immutable raw bytes improves
ordinary verification and acquisition latency without changing the closure.
No proportional speedup is assumed.

The base is private runtime commit
`260d613c36c6ffd665b280002ada6fe83f319e47`. Runtime builder.toml remains pinned
to actual archive
`fa46484cbcf7ce04eabecd31f6a6fc06dc6a7389390276aadc3f22866c731ffb`
and its 1073741824-byte root. That candidate remains unpublished and has no
public-adoption approval. No consumer pin or runtime/spec code changes in
this trial. The existing public artifact is untouched.

The changed build.py source SHA-256 is
`d34b541c96b56da0c30ca4f223cc8e8f12e65121c7ce4c2ca92c3dcd69e031b1`.
Its prior SHA-256 was
`ad434f7ca2686eb1c0a19e6ad721dfa82d859b6b1511b7aaeec91d6d77b34ac4`.
The script records its own source in the recipe. The future factory manifest
must supply the actual recipe, root and archive identities; none is guessed.

Unchanged source SHA-256 values:

| Input | SHA-256 |
|---|---|
| discover/main.go | 28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe |
| pins.json | 9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea |
| packages.tsv | 041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf |
| debian-archive.asc | 312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4 |
| ../crates/pinfold/builder.toml | f74e97ab74b17f180bf30b43bb34048ea82823f833d2bfb139c9f432e835e378 |

The operator owns the fresh exact factory build and must inspect capacity,
inodes and integrity, then compare complete content, owners, modes, links,
xattrs and ACLs with fa apart from filesystem metadata. Helper binary and
software identity must remain unchanged. Fit failure ends this trial; it
does not authorize removing closure content or broadening scope. Any accepted
new bytes then need normal verified offline CLI cold/warm and functional proof.

The worker ran no producer build, VM, suite or latency test. No artifact
publication or public push occurred. Source review and git diff --check do
not establish fit, functional correctness or a performance result.
