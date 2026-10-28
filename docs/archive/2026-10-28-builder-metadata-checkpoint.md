# Builder metadata correction checkpoint

Date: 2026-10-28. Supersedes pending statuses in the immediately preceding
metadata boundary note. No publication or final admission is claimed.

The final scratch/input-limit proof passed. With 633,240,702 bytes of
per-image OCI payloads already stored, the tiny FROM scratch caller build
succeeded. The exact earlier e20 candidate failed on the same catalog with
an input-resource error. The corrected receiver handles the pinned exporter's
null empty layers/diff IDs while retaining equal counts and complete graph
validation. A 512 MiB context file was refused before publication, preserving
catalog bytes; removing that file let the same caller build succeed.
`linux-scratch-limit-proof-1.log` records binary SHA-256
`408af6bbe2565ea3c2f355cb7353603d66aa95973d2c13ab5a1a0aaee4f02f6c`.
The source still carries candidate limits; this is evidence, not approval
of their numeric values or all boundaries.

The corrected Mac caller-build test passed in 71.30 seconds. It verifies
selected-profile freshness and the existing caller/cache/retention cases.
The ONBUILD addition is Linux-only after the native Apple capability result.
`mac-caller-2.log` and `.json` retain its receipt. Mac formatting and locked
all-target Clippy pass; Linux locked all-target Clippy and musl build pass
for the metadata corrections. A final whole-host suite is still required.

The ACL component passed all its checks after the tar-option correction.
The normal CLI parent/child ONBUILD control also passed. Component proof
scope and the difference from ordinary OCI export are recorded separately.

Native discovery still has a source-derived metadata-only external-base
gap. The operator requested a supported-API helper prototype before any
adoption decision. Such a helper could change the bootstrap artifact, so
the old e7dd root must not be promoted prematurely. No new client, build
dependency, permissive source policy or download pin is adopted here.

Remaining admission work includes the helper decision, current-head Linux
and full Mac gates, the CI budget, cancellation/publication and resource
boundaries, approved artifact distribution, numeric policy and independent
correctness/security plus lean/spec/test review.
