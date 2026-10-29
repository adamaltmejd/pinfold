Pinned Linux x64 BuildKit bootstrap for Pinfold image preparation.
This artifact contains a read-only ext4 root and its verification evidence.
It is built without an installed BuildKit or Podman, from a signed Debian
snapshot, exact package closure and pinned BuildKit release binaries.

The full compressed digest names these immutable reviewed bytes. The
manifest records recipe, raw-root and compressed identities separately;
bit-identical filesystem rebuilding is not claimed.

## Caller changes

None for the released Pinfold CLI. The integrated builder consumer remains
subject to its feature PR, resource policy and runtime/review gates.
