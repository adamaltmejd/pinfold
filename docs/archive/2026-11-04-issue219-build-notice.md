# Issue 219: build-start notice

The common image-build path prints its public family and source on stderr
immediately before invoking the builder. Profile, project and caller builds
share this path. An invalid caller name is refused before the notice.

Real CLI checks built three isolated `FROM scratch` fixtures. Each printed
one notice; profile/project stdout remained one image ref, and caller stdout
remained one `built` JSON line. An invalid caller name returned `spec` with
no notice. Only fixture images were removed afterward.

Independent correctness/lean review, format and locked all-target Clippy
passed. No test assertion changed; no prose assertions were added.
