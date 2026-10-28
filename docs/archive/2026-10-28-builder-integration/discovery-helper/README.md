# Native discovery helper prototype

Source-only proposal, 2026-10-28. This does not change the pinned bootstrap,
production integration, dependencies, resource policy, or published artifacts.
It has not been compiled or run by its author.

The executable uses BuildKit's supported `client.Build` callback to invoke
the pinned native `frontend/dockerfile/builder.Build`. It fixes
`requestid=frontend.convertllb`, the native frontend, and linux/amd64.
It supplies no exporter or insecure entitlement and attaches no secret or SSH
provider. It delegates resolver arguments and successful replies unchanged.

Its gateway wrapper records successful `ResolveImageConfig` replies, including
images that become `llb.Scratch` after their config is used. It also records
image replies from `ResolveSourceMetadata`. The output is one bounded JSON
object containing the original structured `convert_llb` result and sorted,
deduplicated `resolutions`. Each resolution records `kind`, `requested` and
`effective` source identifiers, the resolved image descriptor `digest`, and `store`
for an OCI layout when supplied. Config bytes and credentials are omitted.
The original conversion result includes Dockerfile source maps; capture it as
an owned evidence file, rather than printing it into ordinary host logs.

The existing 1 MiB output, 4096 records/operations, 8 KiB identifier and
five-minute candidates are used only for this trial. It does not establish
that every recorded metadata resolution must be leased; the trial must check
that distinction against native frontend reachability.

## Pinned producer inputs

Use BuildKit v0.34.1, commit
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e`, including that commit's `go.mod`,
`go.sum`, `vendor/modules.txt`, and entire committed `vendor/` tree. Add only
this `main.go` at `cmd/pinfold-discover/main.go` in a disposable checkout of
that commit. No new module or module resolution is needed.

The inspected input SHA-256 values are:

| Input | SHA-256 |
|---|---|
| This `main.go` | `d0c43945eff413f9065970597b60f7a3ed1641bfd599410be687c9fd8f2a88fe` |
| BuildKit `go.mod` | `9114ace60db407b7b70eccc340cda6cc5ff3313400afbdd8e1f14849bc244fb3` |
| BuildKit `go.sum` | `35904aa6f9ae03717b2ee03ed55e6f026de822f556e37491801d13cf13fb9e46` |
| BuildKit `vendor/modules.txt` | `d7484cb1c241d6e6a52a2d79e3b96a6c67bae4f41b2a549396844aae7414bcea` |

An earlier prototype at source hash `1ec6ab6d78d952a5090f889a0ce7b806feb33e958b917697fa70c284ed49406f`
compiled successfully with these upstream APIs. The current source adds an
aggregate resolution budget and counts the output newline in its limit;
it requires a fresh compile and fixture proof. The old compile does not
validate this source snapshot.

BuildKit's `go.mod` requires Go 1.26.8. For the trial, use patched Go 1.26.10,
whose Linux amd64 archive is
`https://go.dev/dl/go1.26.10.linux-amd64.tar.gz`, SHA-256
`ccfe7ae822d2c2f41f4fe7c956199967a4dac22d0ee0d2b2bfdb440aab3b20c5`.
The [official downloads](https://go.dev/dl/#go1.26.10) provide this checksum;
the [release history](https://go.dev/doc/devel/release#go1.26.10) records
security fixes after 1.26.8. This is a concrete toolchain proposal, not a
repository toolchain change. Verify the archive before extracting it into
owned producer storage.

From that disposable BuildKit checkout, using absolute paths for the verified
Go installation and result:

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
  /absolute/path/to/go1.26.10/bin/go build \
  -mod=vendor -trimpath -buildvcs=false \
  -ldflags='-s -w -X github.com/moby/buildkit/version.Version=v0.34.1 -X github.com/moby/buildkit/version.Revision=f5b59a1f2fb25a0cb7072c398cab9aed7835da4e' \
  -o /absolute/path/to/pinfold-discover ./cmd/pinfold-discover
```

Retain the verified toolchain archive hash, BuildKit commit/tree identity,
this source's hash, exact build command, `go version -m` output, binary hash
and size, and the trial output. The source commit and committed vendor tree
are the dependency provenance; the helper's own build information names the
Go compiler and build settings. The linker fields identify the embedded
BuildKit version, rather than claiming this helper is an upstream release.

## Trial invocation

Run only against the owned daemon with the existing proxy, namespace,
capability and source-policy controls. It grants no additional network hosts.
The standard upstream local/OCI parsers and `client.SolveOpt` install the
normal filesystem/content session providers. An anonymous Docker auth provider
uses a fresh owned token-seed directory and reads no caller Docker config.

```sh
/absolute/path/to/pinfold-discover \
  --addr unix:///run/buildkit/buildkitd.sock \
  --source-policy-file /input/source-policy.json \
  --local context=/input/context \
  --local dockerfile=/input/recipe \
  --oci-layout base0=/input/bases/0 \
  --opt 'context:localhost/pinfold/image-base=oci-layout://base0@sha256:VALIDATED_DIGEST' \
  --opt image-resolve-mode=pull \
  > /work/helper-discovery.json
```

Repeat `--local`, `--oci-layout` and `--opt` as needed to supply the same
frozen metadata inputs and proxy build arguments as the existing discovery
guest. The source-policy file is mandatory. There is no alternate frontend,
export, secret, SSH or entitlement option. The trusted invocation supplies
the daemon address and frontend options; network authority remains with
the guest's existing proxy and namespace. No progress or
recipe text is parsed. No RUN is solved by the conversion subrequest.

The immediate trial is the real external zero-layer/config-only base: compare
the native LLB source set with successful resolver records, then check a
normal layered external base, a named local OCI base, scratch, ONBUILD and
unreachable stages. Empty-rootfs config use must yield its actual resolved
descriptor identity even when no image SourceOp remains. Keep execution's
default-deny policy unchanged until this is demonstrated.

If adopted after review, place `/usr/local/bin/pinfold-discover` alongside
`buildctl` in a newly pinned bootstrap root. Go and the source/vendor inputs
belong only to that artifact producer; consumers continue to use ordinary
Rust binaries and the pinned bootstrap. That placement and a new bootstrap
publication remain proposals.

Primary API source: pinned `client/build.go`, `client/solve.go`,
`frontend/gateway/client/client.go`, `frontend/dockerfile/builder/build.go`,
`frontend/dockerui/namedcontext.go`, and
`frontend/subrequests/convertllb/convertllb.go`.
