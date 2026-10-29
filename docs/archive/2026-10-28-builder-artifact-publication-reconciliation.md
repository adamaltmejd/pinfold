# Discovery bootstrap publication reconciliation

Date: 2026-10-28. Preparation only. No publication or gate exception.

Current main and proposed producer target:
`5c0ffa2287a51d0fa1a8575181a0a32664b6ef3f`, merged PR 251.
Reviewed/tested producer head:
`784988b022c37f2aaeb2d768046eca14d13802d0`.
Both have the identical complete Git tree
`17bae6a3f308025cbbf1764b42e96d45c874bdcc`.

The audit worktree remains at PR 250 head
`d76ea72ba79fd00027708648f37060226c1dfd3d`. This new record supersedes
its old eight-file `e7dd1ab6...` promotion candidate. It preserves the
earlier dated records and their original gate results. This request
authorizes a local report, not an update/push of PR 250 or publication.

## Exact new candidate

Use the retained directory below, not the old `publication/` directory:

`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/helper-artifact-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/`

The authoritative proposed inventory is the twelve-file table committed at
`5c0ffa2:docs/archive/2026-10-28-discovery-producer/README.md`:

| Full filename | Bytes | SHA-256 |
|---|---:|---|
| Release | 138612 | ed56aac47e7911e65ee63aae8d67e29f20e023f840e49f8ed037043a33de138a |
| Release.gpg | 1760 | 9f80d5c8e4f9fd918a8b4fedd35c00ecc6c1a3ca9eddbc981c83823d4a4b8dc4 |
| SHA256SUMS | 1032 | b8d6c09d0b10f1c61706b587a8ff2d8e27ce05f826906ac9db6c1f46ae8f1a80 |
| buildkit-source-f5b59a1f2fb25a0cb7072c398cab9aed7835da4e.tar.gz | 12143295 | 6608d2ee63bedf99e1e3fecbfa19868c36f34a402c410a9605529986632816c3 |
| debian-archive.asc | 25106 | 312f12f3a3043d9aadc5ba532410dab2b45f91eb76230e7509bf6ced903738d4 |
| discovery-build.json | 6203 | 36084dd639afd6c6532057057a0a65ca990be1681298b2e0282170fd72cd9cd7 |
| discovery-main.go | 6929 | bd5fda8f957fdb9d887a85caffeb6e78ce5c96f380bd52122847b5d6d6ceb335 |
| go1.26.10.linux-amd64.tar.gz | 66936659 | ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5 |
| manifest.json | 9245 | c9643a200f9ff1fd30451c091837f7025a2232c9c909756e3d14ff4f9f6636c6 |
| packages.tsv | 12498 | 041d0dd2bb81bb7d9a73b335f6bbdf4d0160e6d2e682b802d6a5339927f18caf |
| pinfold-builder-linux-x64-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4.tar.gz | 122578909 | 09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4 |
| pins.json | 1443 | 9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea |

All twelve are regular files, totaling 201861691 bytes. SHA256SUMS covers
exactly the other eleven; its own hash is independently fixed above.

The archive contains exactly one regular member, `root.ext4`, 1073741824
bytes, SHA-256
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
Recipe identity:
`d4fea170a1a33385e33951a981e649fc6b07053c5e965b99dc620719e744c30c`.
The recorded helper at `/usr/local/bin/pinfold-discover` is 22282402 bytes,
SHA-256
`0b8927187764740c328a245e21c44a08cb419aa93911d3be624e4c943f2907d0`.
The 1 GiB filesystem size is artifact identity, not approval of a runtime
memory, cache, output or other numerical resource limit.

## Verification performed in this reconciliation

Every retained file was rehashed against the committed table. The original
`bootstrap-helper-evidence.tar.gz`, 199874818 bytes, hashes to
`f38ed6c8b82cbee9a361d09514337304e135e1897ae9827c4361105fec665127`.
Its complete twelve regular members have the same names, sizes and hashes
as the retained directory. The separate 6604-byte component bundle
`helper-api-proof-3.tar.gz` hashes to
`725983f4a29deb079e7eccbf502e211e1c1918cfc6ca2abbeb4c366f0506dbeb`.
Both bundles remain in the durable parent directory; neither is proposed
as a release asset.

The archive's sole member was streamed and hashed independently, and its
gzip CRC/footer/EOF was consumed. No filesystem was mounted or parsed.
The helper binary identity is taken from the existing independent
root/helper receipt and component receipt; this turn did not extract or
execute that helper. Component success is not production runtime evidence.

All five recipe input hashes match merged main. Recomputing their sorted
JSON recipe hash gives the identity above. The helper source sidecar
matches main's `builder-root/discover/main.go`. Manifest and discovery-build
JSON equal their committed evidence copies, and the manifest's helper
record equals discovery-build.json. The retained source archive's go.mod,
go.sum and vendor/modules.txt hashes match the producer pins.

GnuPG verifies the Debian Release using the three pinned primary keys;
package closure and archive keys are unchanged from phase 6a. The live Go
download index confirms stable Go 1.26.10, its 66936659-byte archive and
publisher checksum above. The reviewed Go publication date is October 19;
BuildKit's release tag is dated October 20. The seven-full-day producer
tool rule is unchanged.

The live BuildKit signed-tag record resolves v0.34.1 through tag object
`401bf63d02833b70ef9393eb9702e2b5648dfaaf` to commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e` and tree
`7cb3b9b2864994a620bfdb04c87f56ca8f2f518c`, matching retained receipts.
GitHub reports the signatures verified. The codeload archive hash is a
reviewed measured pin, not a separately published upstream tarball checksum
or independent local signature verification. Preserve that distinction.

The new public sidecars comprise producer inputs, source, build metadata
and public archive keys. Their recorded build paths are not credentials.
A new limited raw-root scan found zero private-key-header, GitHub-key-prefix
or OpenAI-project-key-prefix markers. This is not proof against every
possible secret form; the old root's scan was not reused. Publication
suitability also depends on the reviewed bootstrap recipe and isolated
build. Raw runtime logs, host credentials, proof dumps and caller data are
not in the proposed inventory.

## Concrete proposed promotion

Repository: `adamaltmejd/pinfold` on GitHub. Proposed new tag:

`builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`

Proposed archive URL, not an existing asset or adopted consumer pin:

```text
https://github.com/adamaltmejd/pinfold/releases/download/builder-root-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4/pinfold-builder-linux-x64-09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4.tar.gz
```

Target the full merged-main producer commit `5c0ffa2...`, not `ff6276cb`,
the audit branch, or an unmerged implementation checkpoint. Authenticated
tag and release lookups for this new tag both returned actual HTTP 404 in
this reconciliation. Latest stable remained v0.3.3. Recheck at execution.

Only after audit disposition and Adam's concrete approval of these twelve
assets, this tag and this target:

1. Recheck all twelve regular-file names, sizes and digests against this
   reviewed table, including SHA256SUMS itself. Use twelve explicit paths
   from the retained directory; no glob or old staging directory. Do not
   rebuild or substitute the bootstrap CI output: equal recipe inputs do
   not promise identical filesystem bytes.
2. Confirm repository/account, exact remote target and absence of both tag
   and release. Authentication, permission, timeout and server errors are
   not absence. Stop on an existing object or any ambiguous response.
3. Use the existing one-off `gh release create` approach with the proposed
   tag, `--repo adamaltmejd/pinfold`, full `--target`, `--prerelease`,
   `--latest=false`, reviewed `--notes-file`, and those twelve explicit
   asset paths. The target argument must be the full 5c0ffa2 commit above.
   No CLI v-tag, version bump, pin update, release workflow dispatch,
   clobber, force or asset replacement belongs to this operation.
4. Preserve response IDs and diagnostics. Creation, uploads and publication
   are separate API requests. After any error, interruption or lost
   response, inspect the exact authenticated tag, release ID, draft/public
   state and complete paginated asset list before any further mutation.
   The installed gh flow can attempt release deletion after upload/publish
   failure; that cleanup can also fail. Assume neither success nor rollback.
   Stop for operator review of partial/conflicting state. Do not recreate,
   delete, overwrite or reupload blindly; authorization must cover any
   recovery action. Release immutability settings are not assumed.
5. Verify the public prerelease state and exactly twelve uploaded assets:
   names, sizes, publisher SHA-256 fields and expected download URLs must
   all match the table. SHA256SUMS alone is not authority. Resolve the
   actual remote tag, following any annotated tag object, to the full
   5c0ffa2 producer target. `--target` and target_commitish alone do not
   prove where an existing tag points. Confirm this is not latest stable.
6. Download all twelve files independently into a fresh private directory
   and reverify the table. Recheck the archive's sole raw member, size,
   digest and gzip integrity against the independently reviewed values.
   Missing publisher digests or any mismatch prevents consumer admission;
   retain receipts and investigate without replacing published bytes.

Proposed notes must now name the compiled discovery helper and retained
Go/compiler and BuildKit source inputs. A concise reviewed text is:

> Pinned Linux x64 builder/converter bootstrap with native BuildKit metadata
> discovery. Built from a signed Debian snapshot, exact package closure,
> pinned BuildKit binaries and a helper compiled offline with pinned Go
> against the reviewed BuildKit source/vendor tree. Twelve assets retain
> the root archive and its producer/provenance inputs. Recipe, compressed
> archive, raw root and helper identities are recorded separately;
> bit-identical filesystem rebuilding is not claimed.
>
> ## Caller changes
>
> None for the installed Pinfold CLI. Consumer adoption and runtime controls
> remain separate work.

## Gate evidence and applicability

| Scope/head | Evidence | What it establishes |
|---|---|---|
| Producer 784988b | Retained mac-producer-1.json/log: clean head, exit 0; 24 passed, 1 ignored | Producer E2E test binary passed; 120.75 s test time, 125.95716816606 s measured command time |
| Producer 784988b | Linux CI 40768192635: 25 passed, 1 ignored; 182.19 s test time, 184.682381041 s including Cargo | Actual Linux suite plus format/locked Clippy/AppArmor setup passed |
| Producer 784988b | Bootstrap workflow 40768192630 passed | Producer can build; its emitted bytes do not replace retained 093 bytes |
| Merged main 5c0ffa2 | Complete tree equals producer 784988b | Exact-tree relationship supports applicability of producer evidence to this producer target |
| Audit d76ea72 / executable source tested at 3d6b106 | Mac 23 passed, 1 failed, 1 ignored; exit 101, 438.252 s | Failed audit Mac gate remains failed; separate upstream-429 observation and intentional owned stop causing exit 137 remain distinct |
| Audit d76ea72 | Linux CI 40721053750 and guest-kernel build 40721054088 passed | Earlier audit-head checks, not a new combined-head gate |

The producer Mac receipt's exact command is `cargo test -p e2e --test e2e
-- --nocapture`; do not relabel it as this audit's locked full-Cargo
invocation. Its log includes the live-login test passing. Receipt hashes:
mac-producer-1.json, 234 bytes,
`3a4203e89c442d0425c5c2acde1379987366a90e930195835542d5c2d4a53d75`;
mac-producer-1.log, 6202 bytes,
`4233b07276904d87fe3ee4de36235c522da8497f33cf1d4f37a8073215d31bb4`.
They remain in the durable evidence parent directory.

The operator reports two independent final producer reviews; this bounded
reconciliation also found no new producer-code blocker. It did not repeat
their full review or run a suite, build, VM lease or manual sabotage.
The later producer Mac pass does not retrospectively pass PR 250's run,
validate its different source tree, or grant a gate exception.

## What PR 250 still needs reconciled

1. Supersede its operative eight-asset plan, old digest/recipe/root/bundle,
   ff6276cb publication target and old notes with this newly reviewed
   candidate. Preserve old archives and evidence as dated history. Its
   current public description still describes eight assets; this local
   report does not update it or authorize a push.
2. Account for current-target audit coverage. Main changed 29 paths since
   ff6276cb: producer Python/pins/Go helper, producer docs and evidence.
   No Rust, test assertion, Cargo, workflow, AGENTS.md or skill changed.
   This turn read the changed producer modules whole and refreshed the
   producer contract and provenance; it is a bounded delta review, not a
   newly completed whole-tree release pass. The skill requires the whole
   tree and supplies no automatic artifact-release inheritance exception.
   Default to a current-target whole-tree pass. An operator interpretation
   treating unchanged-file reads as a continuing pass needs explicit
   coverage accounting and disposition, not an inferred waiver.
3. Reconcile pending cleanup cuts. Current main contains none of PR 250's
   five cleanup commits. Do not report them as landed in the publication
   target. Do not rebase/merge/update the audit branch under this request.
   Any later combined source head needs its own appropriate checks and
   independent final-head review; producer success is not that result.
4. Refresh final metrics when that scope is chosen. Current producer main
   versus v0.3.3: 7425 additions / 720 deletions, delete/add 0.09697;
   crates remain 1116 / 695, 23142 Rust lines. Largest Rust source is
   box_.rs at 4368 lines; largest binary source cli.rs at 1304;
   ARCHITECTURE.md is now 1307 lines. These are current-main figures,
   not a measurement after landing PR 250. Guarantee rows remain 27;
   tests are unchanged. Old spec references after the producer paragraph
   move seven lines: G2 to 1243, G9 to 1248, G39 to 1266.
5. Preserve the three written test-proof proposals: live streaming before
   exit, the ordinary HTTP positive-control decision, and scan coverage
   without private filename coupling. No assertion or guarantee was cut,
   and no proposal is fixed by the producer change. Operator/Adam must
   decide their release disposition. Narrowing a guarantee requires
   Adam's substantive spec decision.
6. Resolve the failed audit Mac gate and applicable release-build/slow-gate
   inventory explicitly. No new release build or slow proof ran here.
   If the operator proposes producer-only publication with exact-tree
   producer evidence while leaving PR 250 unmerged/failed, that is an
   explicit audit/gate disposition to review with Adam, not an exemption
   established by this report. No release or merge readiness is declared.

After those dispositions, ask Adam only for the concrete twelve-file
artifact operation and any expressly identified gate exception. Prior
approval to upload the public draft audit did not approve this release.
Exact numeric resource policy remains unapproved and separate. Production
consumer cache/lifecycle/fault work remains with the active implementation
worker; its normal-download, transport, jail/proxy, cancellation, cleanup,
preservation/migration and final runtime gates are not duplicated here.

No source or existing archive was edited. No shared suite, build, VM lease,
Git commit, push, merge, tag, draft release, asset upload or publication was
performed. Only this new local dated report was written.
