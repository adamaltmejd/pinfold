# Builder bootstrap publication, 2026-10-30

The operator authorized publication of the verified 512 MiB bootstrap and
continuation with Ubuntu CI. The bootstrap is published as
[release 428703550](https://github.com/adamaltmejd/pinfold/releases/tag/builder-root-1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd),
a non-draft prerelease whose tag resolves to producer commit
`cf9ea3e3c5e1fb3e8263ec4ff94bb6389a0c1825`.

The operator independently checked all 12 asset names, sizes and publisher
SHA-256 digests against the frozen inventory. A fresh archive download measured
122,074,383 bytes and SHA-256
`1e9a62f3eb034bf8582f4ff5d6e42310b56c5fbc1de201336342647228fe18cd`.
Streaming its sole regular root.ext4 measured 536,870,912 bytes and SHA-256
`1ba38d2c1835623c36479387f60101529848ee8575fe1f5caef164e3a48d89a2`.
Full gzip CRC and EOF verification passed. A separate auditor downloaded and
verified all 12 assets. The operator's receipt is
`.local-evidence/bootstrap-publication-2026-10-30/independent-public-verification.json`.
The latest stable release remains v0.3.3.

This consumer change starts from
`d6c4b59a159dc34ad096bdf3d65543a321d1b32b`. It reconciles the living
documentation with publication and removes the obsolete private-only comment
from builder.toml. Every URL, digest, size and recipe pin value is unchanged.
Code, tests, dependencies and caps are unchanged.

The removed comment still changes derivation identity: job.rs includes the raw
builder.toml bytes in the converter recipe hash. The executable's embedded
bytes and converter derivation identity therefore change. One reconversion of
a previously cached image is expected under the new identity. Fresh Ubuntu CI
must cover it.

Publication verification proves the public bootstrap bytes, not this consumer's
runtime behavior. The operator reports the prior private Linux suite had
25 passes, one host-precondition failure and one ignored test in 522 seconds;
it did not meet the 300-second budget. The prior Mac suite had 24 passes in
85 seconds. Neither run covers the new converter identity. This worker runs
formatting and whitespace checks only, with no build, suite, VM, push or
publication. Runtime merge, a CLI release and numeric or budget approval remain
outside this change.
