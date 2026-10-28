# Interrupted builder output

Date: 2026-10-29 local time. Raw timestamps are UTC on 28 October.
This is a finite manual trial of the normal CLI, not a new test or a
producer substitute.

The executed binary was
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`,
the cancellation-fixed production source at `c4bcd4d`. It used the normal
verified `09393a02...` bootstrap cache.

An external seccomp notification supervisor held the host's real fsync of
the job's received `oci/index.json`. It checked the descriptor's device,
inode and regular-file type, found the CLI's actual Firecracker descendant,
and killed that VMM through its pidfd. It then resumed the fsync without
injecting an errno. The received index was 409 bytes. The kernel receipt
records one kill, VMM death and no supervisor error (`supervisor_errno=0`).
The CLI exited 1; the supervisor propagated that same exit status. There was no
synthetic stream, runtime replacement or host layer extraction.

The CLI reported failed with an incomplete-read diagnostic after 18.525
seconds. Catalog bytes and public references were unchanged. The builder
cache retained its unclean marker. The identical ordinary CLI command,
without the supervisor, then built successfully in 52.057 seconds and
cleared the marker. The full driver took 76.102 seconds.

The supervisor required a private target parent. The owned disposable
VM's builder parent was observed as mode 0775 and changed to 0700 for that
fixture precondition; the permission observation is retained separately.
This does not assert a production parent-mode guarantee. No old Podman
store, managed disk or permanent host setting was changed.

The exact EOF offset is unproven: it could precede a blob header or fall
inside its body. This covers genuinely incomplete output after the index,
not descriptor corruption, hash mismatch, external URLs, late cancellation,
physical crash or host PID-1 reaping. Those boundaries are not inferred
from this receipt.

Evidence is copied out under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.
`truncation-after-success.tar.gz` contains the driver, supervisor source and
binary, raw stdio and kernel/CLI receipts. Its SHA-256 is
`0797cecd0c65eac2d5b204c1980c6d901b55e48802944ea83ec650724496679b`.
The exact production source and binary are already retained in the
successful lifecycle bundle named in the accompanying lifecycle note.

| Input | SHA-256 |
|---|---|
| Supervisor source | `e2e530c1c0915e14ee2a8724263d4ae76f3afc34503f64a3809285bb97618daf` |
| Supervisor executable | `10662147887bd649fe5b885b99be915f8b4f25eda762eeaa29815c1aa238564e` |
| Driver | `3e86528eb95e5ea279339eeff0dd4af49bbe2e75d1234a6f0829a1c2c95de5be` |

Numeric policy, public artifact installation, the full Linux suite and
budget, final Mac validation and admission remain separate gates.
