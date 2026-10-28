# Evidence index and reproduction

These are isolated scratch-trial sources and raw observations for
[the role proposal](../2026-10-28-firecracker-buildkit-builder-trial.md).
They are not shipped code, production APIs, Pinfold E2E tests or an approved
role. Do not run them on a permanent host or shared image store.

## Read the evidence

- versions.json and pins.json: actual host/tool inventory and retained
  final root/artifact hashes. BuildKit's bundled runc is 1.5.2; the apt
  runc package is not the selected worker binary.
- profiles-results.jsonl: initial native builds. OCI export succeeded for
  default/documents, then the absent frames.py helper made those jobs fail.
  native-results.jsonl recovered their artifacts and validated narrowed
  daemon capability behavior; full hit native-cache ENOSPC.
- overlay-results.jsonl: empty-worker default, cached/no-cache repeats and
  full builds. final-results.jsonl: restart reuse, actual RUN proxy controls,
  daemon socket isolation and explicit network.host refusal.
- conversion-results.jsonl: incorrect :latest context key.
  conversion2-results.jsonl: corrected OCI load and metadata, invalid xattr
  fixture and unavailable BusyBox printenv applet.
  conversion3-results.jsonl: successful metadata/ext4 conversion and fixed
  shell Env check; original OCI headers had no xattrs.
- xattr_fixture.py, xattr-fixture.json, xattr-results.jsonl: an independently
  constructed OCI layer with known user.phase6, then successful tar and
  mounted-ext4 preservation. No external-import product feature is implied.
- full-conversion-results.jsonl: fresh no-egress full-image conversion.
  boot-converted-results.json and boot-converted-console.log: separate
  builder-free boot of the received small root.
- frames.py, malformed.py, malformed-results.json: standalone protocol
  model. The case called missing-blobs removes index metadata, not just one
  referenced layer; do not count it as a hostile descriptor-reference test.
- artifact-manifest.json, validated.json, overlay-validated.json,
  xattr-validated.json and objects/: complete observed identities and raw
  selected OCI manifest/config JSON. Binary payloads are local, below.
- proxy.jsonl: chronological trial-proxy decisions across all runs. It is
  not Pinfold's egress-log schema/security validation.

Inspect a job's full raw log, for example:

    jq -r 'select(.name=="run-proxy-controls") | .log' final-results.jsonl
    jq -r 'select(.name=="convert-xattr") | .log' xattr-results.jsonl

Historical plan names are not authority: inventory-narrow in the initial
profiles run still shows the broad daemon. The actual mask is recorded in
each log. Initial bootstrap variants were replaced; only final root bytes
are retained. Final trial-guest.py waits on the daemon's running-server
signal before announcing RPC readiness. Earlier RPC availability alone
was not daemon readiness.

## Durable local artifacts

The complete raw metadata bundle is at:

    /Users/adam/Code/pinfold/target/phase6-builder-trial-2026-10-28/evidence.tar.gz

The exact final bootstrap, input disk, verified downloads and all 18
streamed output files are at:

    /Users/adam/Code/pinfold/target/phase6-builder-trial-2026-10-28/artifacts.tar.gz

These paths are intentionally outside the disposable VM and temporary
folders. Large binaries are not added to Git. The full metadata bundle
also retains repeated kernel console logs omitted from this PR. Git console/
bootstrap logs are whitespace-normalized views; the bundle retains raw bytes. The
artifact manifest/pins permit checking every retained raw member without
mounting a filesystem. Never mount the builder/cache disk on the host;
those writable disks are not in the bundle.

## Reproduce in a new exclusive lease

Read the homelab-trial-vm skill and acquire/release only through labvm.
Copy wanted evidence out before release; release is mandatory on failure
as well as success. Do not take another owner's busy lease.

Use bootstrap.sh inside that fresh disposable Linux VM, with the helper
files alongside it, or restore the exact retained final root and verified
downloads. The recipe rebuild is not expected to match the retained root
hash. For byte-exact inputs use the bundle and verify pins.json. Reconnect
after adding the agent user to kvm. Work under /home/agent/phase6-trial.

Create a fresh cache as an ordinary host file and format it before the
guest uses it. Do not mount/read a cache after guest use. Restore
Firecracker from its verified release archive. Run, for example:

    python3 trial.py overlay.json overlayfs

The resulting OCI identities may change because recipes rerun against live
packages and timestamps. Compare cached builds to their own preceding
uncached build, not to an old golden build identity. Ingest a received
frame stream with:

    python3 frames.py ingest outputs/overlay-full-cold.bin catalog-model

For conversion, use the exact retained fixture/full frames or a newly
validated result. Create the read-only input disk with host mkfs.ext4 -d
from the validator's regular-file OCI layout. Replace each conversion
plan's immutable digest with that validated manifest. Recreate a new
converter work disk for each fresh test. Example roles:

    TRIAL_CACHE=converter-cache.ext4 TRIAL_INPUT=input.ext4 TRIAL_DENY=1 \
      python3 trial.py xattr.json overlayfs

xattr_fixture.py starts from the original validated fixture identity.
Reconstruct that catalog directory from retained outputs/fixture.bin first.
The full conversion needs a 256 MiB input disk and 3 GiB fresh work disk;
small conversion used 64 MiB input and 512 MiB work. No converter attaches
the builder cache.

Native/final historical plans reference guest-side OCI layouts under the
builder work disk. Recreate those through their preceding build/export
steps or copy validated layouts into a guest-owned working area via a
read-only input disk; do not populate them by mounting the cache on the
host. Their plan JSON records exact commands, not a promise that every
historical plan runs against an arbitrary empty cache.

Use boot_converted.py for the received small fixture root, not the full
profile (the latter has no probe-init). It observes markers then stops the
VM. Use malformed.py only against the scratch validator and valid fixture
stream. Neither is proof of a production Pinfold guarantee.

All VM processes started by trial.py are terminated in its finally block.
The trial synced cache data before termination, not a complete production
graceful shutdown protocol. Production recovery/concurrency is an explicit
remaining gate.
