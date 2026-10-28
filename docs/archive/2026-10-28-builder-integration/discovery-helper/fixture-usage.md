# Loopback registry component fixture

Source-only trial fixture, 2026-10-28. No helper, registry or BuildKit daemon
was executed by the fixture's author. Python 3.10+ and its standard library
are enough for the fixture and comparator. The helper's separate producer
instructions still govern the pinned Go/BuildKit inputs.

`fixture.py` starts only a loopback HTTP registry. It launches no subprocess
and makes no outbound request. It creates an exclusive new 0700 work directory
and writes its own files as 0600. GET/HEAD implement the registry v2 ping,
manifest and blob endpoints for exactly two independent images:

- `fixture/zero:latest`: no layers, an inherited ENV and `ONBUILD RUN false`.
- `fixture/layer:latest`: one uncompressed tar layer holding a fixed marker.

The exact generated blobs, manifests and expected identities are retained.
No supplied manifest supplies its own expected digest. The marker layer is
constructed from literal fixture bytes, without extracting another archive.
The HTTP request log records only method and queryless path.

On the exclusively leased disposable host, keep this server alive in an owned
terminal while the parent runs the comparisons:

```sh
python3 docs/archive/2026-10-28-builder-integration/discovery-helper/fixture.py \
  serve --work /tmp/discovery-fixture-new
```

Its flushed `ready` JSON line follows actual socket bind/listen and input
publication. It reports the chosen loopback port. No readiness sleep or retry
is needed. SIGINT/SIGTERM closes this owned server. The parent is responsible
for stopping and reaping any background invocation it starts.

Use a separate owned pinned BuildKit v0.34.1 daemon on that disposable host.
Supply the generated `buildkitd.toml`, a new private state directory and Unix
socket. Select its native snapshotter, host worker network, no containerd
worker and no GC. The registry setting enables HTTP for only the generated
`127.0.0.1:PORT`. The parent supervises daemon privilege, actual readiness,
termination and reaping; this fixture does not add a second supervisor.
This native host-daemon trial does not exercise a VMM, production jail,
Pinfold claims, proxy enforcement or any agent controls.

Use absolute paths to the verified daemon/buildctl and the produced helper.
Clear ambient proxy variables and use the generated empty `docker/` directory
as DOCKER_CONFIG. Do not add an exporter, secret, SSH provider, alternate
frontend, insecure entitlement, network helper or execution fallback. Only
the native conversion subrequest is invoked. The generated source policy
denies external image/HTTP/Git sources and allows only this loopback fixture.
The [pinned source-policy engine](https://github.com/moby/buildkit/blob/v0.34.1/sourcepolicy/engine.go)
applies the last matching allow/deny rule.

For each `CASE` (`zero`, then `layer`), capture stdout and stderr separately.
Replace WORK and the three executable/socket paths with the owned absolute
paths; these commands do not start the daemon:

```sh
DOCKER_CONFIG=WORK/docker /verified/buildctl --addr unix:///owned/buildkit.sock \
  build --frontend dockerfile.v0 --source-policy-file WORK/source-policy.json \
  --local context=WORK/context --local dockerfile=WORK/recipe-CASE \
  --opt requestid=frontend.convertllb --opt platform=linux/amd64 \
  --opt build-arg:TARGETARCH=amd64 --opt image-resolve-mode=pull \
  > WORK/CASE-native.stdout 2> WORK/CASE-native.stderr

DOCKER_CONFIG=WORK/docker /produced/pinfold-discover \
  --addr unix:///owned/buildkit.sock --source-policy-file WORK/source-policy.json \
  --local context=WORK/context --local dockerfile=WORK/recipe-CASE \
  --opt image-resolve-mode=pull \
  > WORK/CASE-helper.stdout 2> WORK/CASE-helper.stderr
```

Require all four commands to exit 0 before comparing their real outputs:

```sh
python3 docs/archive/2026-10-28-builder-integration/discovery-helper/fixture.py \
  compare --work /tmp/discovery-fixture-new \
  --zero-native /tmp/discovery-fixture-new/zero-native.stdout \
  --zero-helper /tmp/discovery-fixture-new/zero-helper.stdout \
  --layer-native /tmp/discovery-fixture-new/layer-native.stdout \
  --layer-helper /tmp/discovery-fixture-new/layer-helper.stdout
```

The bounded comparator requires native/helper image-source sets to agree,
the zero-layer native set to omit an image source, the nonempty native set
to name its actual fixture manifest, and both helper resolution sets to
contain their independently expected manifest digest. It writes a new
`comparison.json`; an existing result is never overwritten. This is a
component observation, with normal CLI, production jail, claims, proxy and
execution-boundary proof explicitly false. It does not authorize helper
adoption, changed networking, a new bootstrap or production policy changes.
