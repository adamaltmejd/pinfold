# Native worktree protection evidence

Omitted from this archive copy: `runtime-*.json`, `processes.txt`, `cleanup.json`,
`verification.json` and `SHA256SUMS` (host identifiers and receipts).

2026-10-15. Read the adjacent report before running these isolated research probes.
They do not implement Pinfold support. Every filesystem mutation targets the
fixed disposable tree `/private/tmp/pinfold-native-acl-20261015`.

## Files

- `probe.py`, `results.json`: real linked worktree, ACL identity checks, aliases,
  host commit, xattr removal, chmod and cleanup.
- `followup.py`, `followup-results.json`: valid POSIX ACL-set control, nested
  ancestor DELETE guards, protected directory case alias, retained descriptor.
- `hybrid.py`, `hybrid-results.json`: eight pointer spellings; a deliberately
  unsafe pre-existing hook hardlink and fixed harmless host marker; new-link denial.
- `eight.py`, `eight-results.json`: eight native pointer masks with ACL.
- `eight_noacl.py`, `eight-noacl-results.json`: final matched native-only pointer
  test, clean ordinary controls, and unprotected nested-ancestor counterexample.
- `eight-noacl-initial-results.json`: earlier run with EEXIST in reused positive
  controls; retained for transparency and superseded by the final run.
- `unicode-results.json`: 27 host-stat checks, not an exhaustive Unicode proof.
- `xattr_probe.c`, `acl_set_probe.c`: static Linux guest helpers. These attempt
  mutations only on explicitly supplied fixture paths.
- `identity-source.md`, `independent-review.md`: source and independent reviews.
- `image.json`: existing image inspected before launches. `runtime-*.json` and
  `processes.txt`: runtime inventory and host identities.
- `cleanup.json`, `verification.json`, `SHA256SUMS`: final cleanup, unchanged
  checkout checks and evidence hashes.

## Reproduce

Requirements: arm64 Mac on case-insensitive APFS, Apple Container 1.5.0 and an
existing compatible `pinfold/profile-default:latest` image, Zig and uv/Python.
No account or helper installation is performed. Image identity is in `image.json`;
a mutable tag must not silently be treated as an exact-version replay.

The fixture ACL principal and default guest identity are Adam / uid501:gid20,
matching the tested host. On another host, adapt these explicit fixture identities
consistently. Do not change real repository permissions. The scripts create raw
runtime boxes; the deliberately unsafe hardlink scenario bypasses Pinfold admission
and is not evidence of an admitted Pinfold exploit.

Set `EVIDENCE` to this bundle. Require the scratch path to be absent. The probes
run in order because later probes use earlier disposable fixtures.

```sh
B=/private/tmp/pinfold-native-acl-20261015
EVIDENCE=<checkout>/docs/archive/2026-10-15-native-worktree-protection
test ! -e "$B" && mkdir "$B"
cp "$EVIDENCE"/*.py "$EVIDENCE"/*.c "$B/"
export UV_CACHE_DIR="$B/uv-cache"
export ZIG_GLOBAL_CACHE_DIR="$B/zig-global" ZIG_LOCAL_CACHE_DIR="$B/zig-local"
zig cc -target aarch64-linux-musl -static -O2 "$B/xattr_probe.c" -o "$B/xattr_probe"
uv run --no-project --python 3.13 "$B/probe.py"
zig cc -target aarch64-linux-musl -static -O2 "$B/acl_set_probe.c" -o "$B/tools/acl_set_probe"
uv run --no-project --python 3.13 "$B/followup.py"
uv run --no-project --python 3.13 "$B/hybrid.py"
uv run --no-project --python 3.13 "$B/eight.py"
uv run --no-project --python 3.13 "$B/eight_noacl.py"
```

Each probe removes its exact VM and clears its own ACL entries in `finally`.
The final probe also restores its deliberately renamed fixture ancestor after
normal completion. Inspect output after failures; do not prune unrelated boxes.
All names begin `pf-native-acl-`; all have label
`dev.research.owner=native-acl-20261015`.

The scripts are bounded observations, not production tests. Most record failure
statuses rather than asserting a particular result. Verify the final ordinary
control has exit0 and empty stderr; every protected attempt must fail for the
expected filesystem reason with bytes and mode unchanged. The nested-ancestor
rename is deliberately expected to succeed, demonstrating the remaining gap.

After retaining results, verify no owned boxes remain, clear any remaining ACLs
only on these owned fixture inodes without following symlinks, and remove the
scratch tree and isolated Zig caches. `chmod -N` is appropriate only here because
the fixture had no pre-existing user ACLs. A production lifecycle must preserve
user ACLs, coordinate concurrent boxes and recover from crashes.
