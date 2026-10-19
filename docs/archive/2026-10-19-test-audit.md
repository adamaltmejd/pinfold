# Test audit: three observer gaps

Source: the 2026-10-14 performance and safety review, which named observer
weaknesses in existing tests. Each was rechecked on main at `59c1612`. All
three were real. Each fix stays inside its test. No row, test or binary line
changed.

| Row | Block | Gap | Disposition |
|---|---|---|---|
| 6 | `pi.rs`, host `ps` check | A failed `ps` prints nothing, so the absence check passed whatever argv held. | rewrite: require `ps` success first. |
| 23 | `box_.rs`, per-build identity loop | A missing ref, a missing latest and a null built id compared equal as `None == None`. The third build had no other check. | rewrite: the runtime must resolve the ref; latest and the built id must equal that id. |
| 5 | `box_.rs`, `unshare -U` refusal | A host policy refusing user namespaces would pass the refusal with pinfold's seccomp restriction removed. | rewrite: on Linux, the same image under podman's default profile must create a user namespace. |

Sabotage, as named in the comments: 6, pass the secret in the runtime's
argv; 23, drop the build's tag arguments; 5, the existing seccomp sabotage,
now attributable on any host.

Lines: tests +28 / -3, binary 0.

Not fixed, out of this scope: the review's cleanup helper finding in
`box_.rs` (directory-read errors read as an empty directory).
