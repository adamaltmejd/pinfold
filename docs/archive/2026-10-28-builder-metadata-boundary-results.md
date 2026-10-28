# Builder metadata boundary results

Date: 2026-10-28. Evidence directory:
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

Normal Linux CLI reproduced inherited ONBUILD execution during conversion
at `a75e041`. The converter now derives a temporary config without inherited
triggers and rehashes its config/manifest/index. Original catalog OCI bytes
and layers stay unchanged. The corrected parent built successfully; the
child then executed its inherited RUN false in its actual build. Logs:
`linux-onbuild-before-2.log`, `linux-metadata-fix-proof-1.log`.

The independent metadata component initially lost access ACLs. Pinned GNU
tar applies raw ACL xattrs, then its --acls option replaces them with mode
ACLs when textual SCHILY.acl headers are absent. BuildKit's fsutil exporter
emits raw SCHILY.xattr attributes. Removing --acls fixed this format mismatch.
The second component run passed ownership, modes, hardlinks, symlink,
whiteout, capability, access/default ACL and user-attribute checks through
the production converter and agent. `linux-acl-component-1.log` records the
failure; `linux-acl-component-2.log` records success. Its receipt and original
expectations remain on the owned VM pending durable evidence collection.

This proof bypasses the outer host CLI, jail, claims and production receiver.
It establishes converter toolchain fidelity, not those other boundaries.
Ordinary RUN-created ACLs already disappear in the pinned OCI exporter;
that earlier observation is separate from converter fidelity.

The real catalog was populated with 632,137,100 bytes of per-image OCI
payloads. The exact older `e20dae3` candidate then refused a tiny build as
resource-limited because it copied unrelated images. `linux-unrelated-before-1.log`
records its binary digest and failure. The current native-discovery path
reaches the tiny build without that refusal, revealing a separate scratch
output decoding bug: the pinned exporter serializes zero layers and zero
diff IDs as null. Decode those as empty while retaining equal counts and
all descriptor/hash/config guards. Final positive verification is pending.

An earlier Mac caller test reached the new child check but its native
builder omitted ONBUILD execution. The new regression is scoped to Linux;
no Apple parser or behavior change is introduced. Its rerun is pending.

## Caller capability detail

Linux discovery and execution use the pinned native Dockerfile frontend.
A recipe's #syntax cannot select another frontend. This describes the new
implementation; it does not assert that the previous Podman backend
supported arbitrary frontend selection.

The uncompiled Podman adapter and obsolete runner setup are removed. Old
stores, derived roots and agent disks remain outside migration scope.
Publication, numeric resource policy, complete runtime gates and final
independent review remain required. No artifact promotion is authorized.
