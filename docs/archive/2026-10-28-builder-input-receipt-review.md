# Builder input receipt review

The user approved one public base-field rule for macOS and Linux: only
managed project builds record the selected local profile identity. Caller
and profile builds report null/empty. Resolved image inputs remain internal.

The operator approved removing the unused attestation parser. The candidate
now records bounded native discovery inputs, freezes external image digests
with a default-deny execution source policy, and leases only selected local
images. These receipts are metadata, not independent provenance evidence.
The receiver admits one direct runnable manifest. It refuses attachments,
nested indexes, unknown media, external URLs and unreferenced objects while
retaining size, hash, config, platform, label and complete-graph validation.

This replaces the earlier attestation design and removes its parser rather
than retaining a compatibility path. Converter derivation includes the host
recipe, guest helper, shared init preparation and pinned kernel/VMM manifest.

## Evidence at this checkpoint

- Mac formatting and locked all-target Clippy pass for this source.
- Earlier real Linux CLI runs completed discovery, build, separate conversion,
  publication, local ARG/stage/COPY builds and launch with the builder absent.
- Guarantee 44 passed once on the disposable Debian host. It is partial
  evidence: that host lacks Ubuntu's required AppArmor control.
- One earlier full Mac suite passed 24 tests with one existing ignored test.
  A later run found a new profile-freshness fixture ordering error; it is
  fixed but has not been runtime-verified again.
- Ordinary OCI export preserved ownership, modes, links, whiteouts and file
  capabilities. ACL and user xattrs were already absent from BuildKit's OCI
  output, before conversion. A separate component proof remains necessary.
- No final full Linux suite, final Mac suite, resource-limit approval,
  cancellation/publication race proof or final independent approval is claimed.

The numeric candidate limits are absent from the binding spec. The earlier
checkpoint's statement that they had been removed was premature; this source
has now been checked directly. Constants remain unapproved candidates.

No assets, release, tag or PR have been published. The old Podman store and
managed disks remain outside migration scope. Artifact-only publication is
still an operator/human gate.
