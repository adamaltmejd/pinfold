# Builder CI artifact prerequisite

Date: 2026-10-29 local time. This records existing publisher metadata.
No workflow, artifact publication, tag or release was started.

Producer PR 251 merged as `5c0ffa2`; its inputs match the runtime branch.
The successful producer workflow run `40768192630`, job `123097962447`,
built source `784988b022c37f2aaeb2d768046eca14d13802d0` and uploaded the
normal `builder-root` Actions artifact, ID `12851321790`. That wrapper is
200,022,365 bytes, publisher digest
`f34d43839c91abf4a7260fc6eb3d94f0c926d8d060ccdd0d77ade52cff9d85f7`.

Its printed producer manifest records archive
`b115df4a7a8f368368fd54500344eac688a0ac6b6e7e3b3ffdf8a5067943be9e`,
122,584,632 bytes, and 1 GiB raw root
`6d2a73b3bdb196a1e76db119438c08d34fab91f1698c2506e6b79a63fdeef00a`.
The helper binary hash agrees with the reviewed helper. This filesystem
is nevertheless different from the root tested by the runtime candidate.
The producer explicitly does not promise bit-identical filesystem bytes.

The runtime embeds independently reviewed archive
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`,
122,578,909 bytes, and raw root
`ef1aaa0fe1c6cafaa57bd4615ed6a077fe2cd27adc4a114792401bf6d1de1968`.
The existing CI artifact cannot populate those pins. Do not silently
rebuild, rehash the pin, or add a test-only artifact source to make CI pass.

The concrete prerequisite is an authorized immutable publication of the
already reviewed 093 archive, followed by a reviewed HTTPS download pin
and a normal fresh-cache installation. Current manual cache population
does not satisfy that gate. The earlier publication proposal names older
phase 6a bytes; it cannot authorize publication of this newer helper root.
The ordinary Ubuntu CI workflow currently has no source for the 093 bytes.
Its runtime result must remain pending until that prerequisite is met.

Publisher metadata and the manifest extracted from the existing job log
are retained as `ci-bootstrap-40768192630-manifest.json` under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
The wrapper digest is publisher metadata, not an independent local rehash
of an Actions download. No production fresh-cache success is claimed.
