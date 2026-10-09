# Linux plain-file compatibility check

2026-10-09. The user requested a less risky follow-up to the blocked Linux
security probe. This check deliberately covered ordinary reads, writable
scratch files, reported mount options and clean teardown. It did not
attempt protected writes, aliases, replacement, Git metadata changes or
host execution. It does not complete Y-10's security gate.

## Environment and setup

- Disposable homelab VM, Debian 13.7 at completion, x86_64, ext4.
- Running kernel `6.12.107+deb13-cloud-amd64`.
- Podman 5.4.2, rootless, systemd cgroup manager, cgroup v2.
- Published Pinfold 0.2.2 x86_64-musl binary, verified against release
  SHA256SUMS. SHA256:
  `7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.
- `docker.io/library/alpine:3.22`, image ID
  `c83674e1999044d33d751661371b873539f47e5b5c5ca3320c7e0377acca6238`.

The fresh VM lacked Podman. Its existing package operation was allowed to
finish; standard Podman, uidmap and user-session D-Bus packages were then
installed. The agent user received a systemd user session and linger for
the trial. No kernel security restriction was disabled. The entire VM is
disposable and reset on release.

## Observations

Each real `pinfold box up` had no egress, one CPU and 256M memory. A fresh
plain-text project directory was exported read-write at `/workspace`;
its `reference` subdirectory was exported read-only at
`/workspace/reference`. A sibling `scratch` directory was writable.
The script waited on the CLI's `ready` event before using `box exec`.

Both recorded runs:

- Read the reference fixture's expected text from the guest.
- Wrote only to the designated scratch directory; the host read the exact
  expected text afterward.
- Observed `rw,nosuid,nodev` for `/workspace` and `ro,nosuid,nodev` for the
  nested reference mount in the guest's `/proc/self/mountinfo`.
- Left the reference fixture's bytes unchanged.
- Closed owner stdin and observed owner exit 0.

Readiness took 0.633 seconds and 0.494 seconds. The image was already
pulled, the runtime was warm, and no harness or agent was launched. The
project contained only tiny fixture files. These are two observations,
not a startup benchmark or evidence about large repositories.

The first version of the observation script completed its two ordinary
file runs, then failed because its final `box list` query omitted the
required label. Both owners had already exited 0. The script was corrected
to label its boxes and select that label. The recorded run then completed
successfully. A final independent `podman ps -aq` count was zero.

The observation script and complete recorded JSON were saved in the local
temporary files `/private/tmp/pinfold-linux-compat.py` and
`/private/tmp/pinfold-linux-compat-result.json` before VM release. The release
completed successfully and reported `status: idle`.

## Limits

Mount flags report the configured view; this check did not try to defeat
that view. Unchanged reference bytes after reads and unrelated writes are
not proof that writes through other names would be refused. No hardlink,
symlink, case-alias, duplicate-export, protected-path replacement or
concurrent-host-mutation case was exercised. No Git worktree was used.

The blocked adversarial probe was not repeated. Y-10 remains parked and
Y-9 remains dependent on it. No production code, test assertion, security
contract or linked-worktree refusal changed.
