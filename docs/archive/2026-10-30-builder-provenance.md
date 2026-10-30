# Linux builder evidence provenance, 2026-10-30

The companion [decisions](2026-10-30-builder-decisions.md) and
[validation](2026-10-30-builder-validation.md) consolidate PR252's archive.
Every original byte remains in immutable reviewed Git commit
4f87c982bbf0c1a13f29a22f0e3278774d32fd2f. The
[preservation manifest](2026-10-30-builder-originals.sha256) lists all 67
original repository paths and SHA-256 hashes, including removed trial scripts.
No runtime source, test, workflow, pin, spec or README change is part of this
compaction. Earlier records remain dated observations, not current promises.

## Recovery and preservation

Read an original directly from Git without changing the checkout:

    git show 4f87c982bbf0c1a13f29a22f0e3278774d32fd2f:<path-from-manifest>

The same originals are available in the [immutable archive tree](https://github.com/adamaltmejd/pinfold/tree/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive).
Proposal commands, prototype Go/Python code, superseded asset tables and
intermediate reviews need not remain executable files in the current tree.
Their exact source, failed outcomes and finer-grained identities remain
recoverable through this commit and the manifest.

Local operator backup root:
/Users/adam/.codex/worktrees/ed8e/pinfold/.local-evidence/pr252-evidence-compaction-2026-10-30/.

| Preserved material | SHA-256 |
| --- | --- |
| original-pr252-archive.tar.gz: all 67 files,5800 lines,322715 bytes | c78fc68725d74af983c9541704422e7419e2fd42cd9ca62dff9c15615399e638 |
| before-manifest.json: original paths, hashes, bytes and line counts | 3efc706538930ec68fd9d6c2087dcdc2ca76caeeb1fd00d1e87504520a33865b |
| original-operator-admission.tar.gz: six later approval/source-assurance/final-result records | 3938a26d49edc7b6151057fb5878b903eb0c4338e505afc1539f5e12bb48b2a5 |
| normal-cache-and-catalog-hashing-proof.md: later d6c operator observation | 4701d5f14019a8ff43626e876477ed55e1abfcf1f3b5c63e8e2e6d84f73b205c |

Every original archive member was independently checked against the reviewed
tree and manifest. The six operator admission records postdate 4f87 and were
preserved separately; they are not falsely attributed to that Git snapshot.
Their per-file hashes are in operator-admission-manifest.json.
The separate d6c note records exact Linux release
ece4445a067a01e70c9c004467ae256c28cc96a8b109aef1b4c1f201e73acfd0,
profile UUID reuse/refresh and completed-image corruption/restore observations.
Its raw d6c-cache-integrity-receipts.tar.gz is retained under the operator's
.local-evidence/one-builder-latency-2026-10-29/bootstrap512/.
Its independently rechecked SHA-256 is
56b0fc95036ebd02283d451bac2426d63eda1860f5900ca4876da0dcd84ec77b.
The non-archive tree fingerprint (mode/type/blob/path, NUL-separated) at 4f87:
bde57a2ddbb52a57b15d1a81f05214b6e1bdee463f050dc8dccc69680a455a1c.
An exact comparison must retain all 111 protected entries unchanged.

## Approved bootstrap

The non-latest [1e9 prerelease](https://github.com/adamaltmejd/pinfold/releases/tag/builder-root-1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd)
is release 428703550, targeting producer
cf9ea3e3c5e1fb3e8263ec4ff94bb6389a0c1825. The independent artifact auditor
downloaded and verified all 12 assets; parent independently checked metadata, fresh archive/raw root,
full gzip CRC/EOF, actual tag target and unchanged latest stable v0.3.3.
GitHub immutable=false; embedded archive digest remains installation authority.
No repository-setting change, CLI release or replacement of old 093 occurred.

| Object | Bytes | SHA-256 |
| --- | ---: | --- |
| Published gzip archive | 122074383 | 1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd |
| Raw root | 536870912 | 1ba38d2c1835623c36479387f60101529848ee8575fe1f5caef164e3a48d89a2 |
| Discovery helper executable | 22294690 | d038bd1623499d5ef47b1aec9659854b72eed48ae2f7d066ee6f0c0a358cc738 |
| Producer recipe | — | edac709b3e4c4dcac3d4edeb80272fc2f052fe19d1370e245e61f1d4a79daa07 |
| Discovery helper source | — | 28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe |

The 512MiB factory changed geometry, not packages or runtime caps. Fresh
comparison covered 6480 nodes and metadata/hardlinks/xattrs/ACLs, with one
4364-byte ldconfig auxiliary-cache content exception. It is not byte-identical
filesystem proof. The retained e2fsck -fn receipt reports a clean filesystem; 207.96MiB and
26279 inodes were free. No unretained exit code is reconstructed here. The helper was unchanged. A comment removal from
builder.toml changed converter recipe identity despite unchanged pin values.

Producer manifests are evidence, not pin authority. Publisher HTTPS source
hashes/signed-tag records are not independent tar-checksum or local-signature
verification; Go's publisher checksum is separately recorded. Recipe identity
does not promise bit-identical mkfs output. Earlier e7dd/fa/093 and CI b115
bytes identify their own trials and must never substitute for reviewed 1e9.
The exact12-asset inventory and finer source-build identities remain in the
public release and preserved producer/publication-verification manifests.
The [publication record](https://github.com/adamaltmejd/pinfold/blob/4f87c982bbf0c1a13f29a22f0e3278774d32fd2f/docs/archive/2026-10-30-builder-bootstrap-publication.md)
locates the parent's .local-evidence/bootstrap-publication-2026-10-30/
independent-public-verification.json receipt. Publication is complete;
do not repeat it.

## Older raw receipt bundles

The table records hashes from the original evidence, not new reruns or new
rehashes of every old bundle. Common durable root:
/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/.
The validation table links immutable notes containing exact source/binary,
driver, object and individual log hashes. Historical 093 is not current 1e9.

| Bundle | SHA-256 | Scope |
| --- | --- | --- |
| helper-api-proof-1.tar.gz | 54591a9f95b42906ca00f06cfb25d473ea1041ff344ea1a0d001cc5d3b72f6ba | Native resolver component |
| helper-api-proof-2.tar.gz | c4a550fb4ca21299eb55fea65c0e1a40dac313d9204a03b1ade148eb186590d2 | Aliases/ARG/COPY/scratch component |
| bootstrap-helper-evidence.tar.gz | f38ed6c8b82cbee9a361d09514337304e135e1897ae9827c4361105fec665127 | Historical 093 producer |
| ready-reuse-proof.tar.gz | f7c5a9ecc3c8c903b7f358621da9d4c18eb973717818d10fb9cb16f280f26908 | CLI launch with builder absent |
| integrated-smoke-image.tar.gz | 21c3ab46fd8efc531ed1a70c3d8ac0e3a024c0783e0b296c2b10365772b039a6 | Earlier CLI output |
| local-zero-proof-partial.tar.gz | a89f5a15f54b8192d713d2ca65e46a3715b06db3c0abd9e11cd352fd565c15cb | Partial; aggregate driver failed |
| durability-continuation.tar.gz | 77695e60a22947f8f7c158415e304912fe05297186a2b552a6aca1b8391dc4ef | Postcommit EIO and launch |
| runtime-delta-tested-source.tar.gz | 74dea8067c05c415a04ab7be44c2b9ea5d2e80010c58f4100c649b9637b21107 | Earlier tested-source association |
| current-root-metadata-component.tar.gz | 1fbc569fd83b2e25b46874643ebf045f75926144397bf3e478b95ffba5c073da | Component only; ACL/export limits remain |
| current-builtin-profiles.tar.gz | c780de62a84cd1449e95891315e897c0ff845f52e758d141f3fc0de85c410d89 | Historical documents/full fit |
| cache-failure-before.tar.gz | 4cc3484179b8824244f16eff95112b1519943d999191a1e07b6ac6deef9f2cae | Failed cache-reuse control |
| cache-failure-after.tar.gz | 2574a80ce9041dd4defbefd14e086ed5d30be679e0bbedb0932ecd41fead1946 | Corrected initial reuse sequence |
| truncation-after-success.tar.gz | 0797cecd0c65eac2d5b204c1980c6d901b55e48802944ea83ec650724496679b | Received-output interruption, not selected handoff |

The compact API bundle whose original note omits its basename is identified
there by 725983f4a29deb079e7eccbf502e211e1c1918cfc6ca2abbeb4c366f0506dbeb;
no filename is invented. Earlier phase6a evidence at
/Users/adam/Code/pinfold/target/phase6-builder-bootstrap-2026-10-28/evidence.tar.gz
has SHA-256 f8681084207eabc7004667342bd97df650d2ee101aa365d8dd75965a03dbd97b.
All other raw-ref maps, source manifests, phase traces and rejected proposals
remain precisely locatable through the complete preservation manifest.
