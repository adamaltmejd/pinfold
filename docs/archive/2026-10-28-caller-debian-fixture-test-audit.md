# Caller Debian filesystem fixture reduction

Date: 2026-10-28. Test-audit approval review of the bounded row 23 fixture
revision. It preserves the existing host assertions and changes no test
function or production path. The revised source has not been run.

1. **Guarantee row.** Linux retains the zero-layer local profile's config,
   the first build's real post-commit EIO supervision, all built/ref/latest
   assertions, marker launches, ONBUILD parent/child checks, failure-tail
   checks and cached/uncached stamps. Its filesystem now comes from the
   fixed Debian image below. One nonempty file still comes from the full
   local default image through COPY. Apple continues to use that default
   image for its recipes. The existing receipt eprintln is preserved.
2. **Named sabotage.** The added comment names ignoring COPY from the
   nonempty local image. Only that COPY writes `/local-base-version`, so
   the following real RUN's `test -s` fails and the first built assertion
   fails. The existing local config, ONBUILD, cache, publication and
   selected-profile sabotages remain. No sabotage was executed here.
3. **Prior gap and hardest case.** Replacing all local filesystem input
   with an external image would lose nonempty local COPY coverage. The
   caller therefore combines a local config-only FROM, an external Debian
   filesystem COPY, and a one-file local COPY in one build. The child
   ONBUILD build also retains its nonempty local FROM. The local zero-layer
   context still has a SourceOp; this does not replace the separate manual
   proof of an external image being optimized to scratch and losing one.
4. **Outside expected values.** `PROFILE_PHASE=two` remains a literal from
   the host-written profile fixture. The recipe names the default Debian
   toolchain's `/etc/debian_version` as a nonempty COPY input and a distinct
   destination that the external filesystem COPY does not populate.
   Expected marker bytes, EIO and public ref agreement are unchanged.
   The Debian identity was independently verified against publisher HTTPS
   outside Pinfold; it is not derived from Pinfold's output.

The fixed input is
`docker.io/library/debian@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4`.
It is the existing Debian image family required by built-in default
preparation, and this index already appears in the current built-in profile
receipts. This adds no image family or registry endpoint.

Public metadata is retained under
`target/phase6b-builder-integration-2026-10-28/debian-fixture-primary/`:

| File | Bytes | SHA-256 |
|---|---:|---|
| `index.json` | 8973 | `f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4` |
| `manifest-linux-amd64.json` | 1021 | `1e1a22175fbf1b32157ddaf0219e1db4e5401fab2b94ed285add64d18587cd2d` |
| `config-linux-amd64.json` | 451 | `394ffa2d2ac8f5c9f45148f22bad3587429bceb19c93b88e8595f1e3f7892853` |
| `receipt.json` | 2503 | `473378e4f96c8d67d561ec20a03bd61b7df36a678be4a6a6a38b8eee2f1d3922` |

The byte hashes and descriptor sizes match the publisher metadata. The
registry's digest headers match for index and manifest. No layer was
downloaded or unpacked, so this metadata does not prove shell, coreutils
or source-file contents. The revised real RUN and box observations must
establish the commands and copied file actually work in this fixture.

The fixed index plus the production linux/amd64 platform prevents a moving
tag from invalidating the repeated-build identity assertion. The literal
index is an explicit fixture maintenance input: changing it requires a
reviewed public metadata receipt. It need not follow each built-in default
refresh, and a future default build is not guaranteed to have pulled this
same immutable index already.

`default_image(&env)` remains under exclusive preparation and still warms
the shared suite cache. Its result is kept as `default_base`, passed to the
freshness helper and used for the one-file local COPY. The default helper
builds once per suite cache. No CLI build or box was added. Linux's failing
recipe and stamp profile use the same fixed small base. Existing guard
lifetimes and the atomic downgrade after first-build checks remain.

The earlier `linux-row23-local-1.log` records **0 passed, 1 failed** in
**340.52 seconds** at the unchanged-input image-id assertion. Its SHA-256 is
`6ec6a2bd5ad861beac279c91c2f283245fef1c1d6fb84a8adb681df522fb30e7`.
This is failed earlier evidence, not a performance pass or proof that the
new fixture resolves that identity failure. Final runtime and suite-budget
measurement remain with the parent session.

The scoped Rust edit is 12 added / 3 removed lines in row 23: two sabotage
comment lines, the base selection, and the two-source COPY recipe. It adds
no helper or dependency. Only row 23's spec clause and this new audit note
accompany it. No production source or historical archive was edited; no
test, build, VM or git operation was run for this revision.
