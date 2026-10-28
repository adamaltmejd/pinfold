# Empty external image discovery limitation

Source review only. No normal CLI reproduction has been collected.

Pinned BuildKit v0.34.1 can replace an external image with an empty rootfs
by `llb.Scratch`. Its configuration may still affect the Dockerfile, but
the native `frontend.convertllb` response then omits the image SourceOp.
The candidate's default-deny execution policy safely refuses that image's
second-pass metadata lookup because discovery supplied no pinned target.
It does not permit an undiscovered fetch or invent provenance metadata.
Local OCI contexts retain their source operation and are not this case.

The conversion response contains marshalled operations, operation metadata
and source maps. Outline, targets and lint do not expose the resolver's
resolved image identities either. Progress text and cache contents are not
an adequate structured per-request source receipt.

The supported BuildKit Go client has `SourcePolicyProvider`, with structured
metadata requests, returned digests and conversion decisions. A bounded
helper could observe and pin metadata-only inputs without parsing recipes
or executing RUN. Plain buildctl exposes no equivalent provider option.
Such a helper would add a production executable and pinned build dependency;
it is not part of this source checkpoint. An upstream structured discovery
extension is another possibility. Do not broaden execution's allow rule to
conceal this gap.

Primary sources:

- [Scratch optimization](https://github.com/moby/buildkit/blob/v0.34.1/frontend/dockerfile/dockerfile2llb/convert.go)
- [Native conversion response](https://github.com/moby/buildkit/blob/v0.34.1/frontend/dockerfile/dockerfile2llb/convertllb.go)
- [Subrequest dispatch](https://github.com/moby/buildkit/blob/v0.34.1/frontend/dockerui/requests.go)
- [Policy provider](https://github.com/moby/buildkit/blob/v0.34.1/sourcepolicy/policysession/provider.go)
- [Policy protocol](https://github.com/moby/buildkit/blob/v0.34.1/sourcepolicy/policysession/policysession.proto)
- [Solver handling](https://github.com/moby/buildkit/blob/v0.34.1/solver/llbsolver/policy.go)

Before calling arbitrary caller-image compatibility complete, reproduce
the metadata-only external-base case through the normal CLI and resolve or
explicitly adjudicate the capability gap.
