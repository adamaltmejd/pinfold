# Converter inherited ONBUILD regression

Date: 2026-10-28. Before-fix source: `a75e041`.

The normal Linux CLI reproduced this bug on the Debian trial host. A caller
image containing `ONBUILD RUN false` finished its OCI build, then its separate
converter executed `[1/2] ONBUILD RUN false` and failed. The trial log is
`linux-onbuild-before-2.log` in the parent integration evidence bundle.

The fixture uses the already prepared normal CLI smoke image. Choose fresh
caller names if these names already exist. No test seam or imported image is
needed.

```sh
mkdir -p /tmp/pinfold-onbuild/context
cat > /tmp/pinfold-onbuild/parent.Containerfile <<'EOF'
FROM localhost/pinfold/image-smoke:latest
ONBUILD RUN false
EOF
pinfold image build onbuild-parent \
  --containerfile /tmp/pinfold-onbuild/parent.Containerfile \
  --context /tmp/pinfold-onbuild/context
```

The parent's intended result is exit 0 with a completed image. Its ONBUILD
instruction defines future build behavior; it must not run during conversion.
Before the fix, this command exits nonzero in the converter.

After fixing conversion, this child is the control that the stored original
image still contains its trigger:

```sh
cat > /tmp/pinfold-onbuild/child.Containerfile <<'EOF'
FROM localhost/pinfold/image-onbuild-parent:latest
EOF
pinfold image build onbuild-child \
  --containerfile /tmp/pinfold-onbuild/child.Containerfile \
  --context /tmp/pinfold-onbuild/context
```

The child must fail its actual build because the inherited `RUN false` runs
there. The fix creates a private converter config without ONBUILD metadata,
rehashes that config and its manifest, and points only the converter's local
index/job at it. Stored OCI configuration and layer bytes remain unchanged.
Both exact and case-insensitive config/OnBuild keys are handled because the
pinned Go frontend accepts those aliases. No layer is unpacked on the host.

Pinned BuildKit v0.34.1 source explains the distinction:
`frontend/dockerfile/dockerfile2llb/convert.go:562` initializes inherited
ONBUILD commands, and `:1325` prepends them to the stage. Inherited WorkingDir
uses `dispatchWorkdir(..., false, ...)` at `:813`; directory creation is inside
the `if commit` branch at `:1543`. The tar exporter mounts the whole reference
through `exporter/local/fs.go:139` and does not use WorkingDir to select output.
WorkingDir alone therefore does not change the converted filesystem.

The after-fix parent/child runs remain to be recorded. No new automated test
was added before collecting that runtime evidence.
