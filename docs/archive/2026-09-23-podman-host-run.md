# Rootless podman host run, 2026-09-23

Historical record of the operator's end-to-end run of the existing suite on a
rootless podman host, and the two adapter defects it found. The current spec is
`docs/ARCHITECTURE.md` and wins where they differ. The candidate run was
`0c7c370` merged with main.

## Host

- Debian 13 amd64, podman 5.4.2, crun, rootless with the systemd cgroup
  manager and linger enabled.
- Preflight refusals behaved as specified: with `cgroup_manager = "cgroupfs"`
  in the user's `containers.conf`, `box up` refused and named cgroupfs.

## `--dns none` conflicts with `--network none`

- **Observed.** With both flags, podman 5.4.2 refuses every `run`:

  ```
  Error: conflicting options: dns and the network mode: none
  ```

  The box exits 125 before ready, so every test fails.
- **Observed.** Without `--dns none`, podman writes its own `resolv.conf`:
  `1.1.1.1` on this host, and the host's nameservers where they are not a
  loopback stub.
- **Fix.** Drop `--dns none` and bind an empty file pinfold owns, read-only,
  over `/etc/resolv.conf`. The box has no network and no resolvers; the proxy
  socket is its only way out. With this one change, every existing test in
  `crates/e2e/tests/box.rs` and `crates/e2e/tests/pi.rs` passed on this host.
- **Spec.** `docs/ARCHITECTURE.md` now says "an empty read-only
  `/etc/resolv.conf`" in the podman line, and the `--no-hosts` / `--dns none`
  open question is resolved.

## The prepended seccomp rules had no effect

- **Observed.** The candidate prepended `SCMP_ACT_ERRNO` rules for `clone`,
  `clone3` and `unshare` to podman's default profile. The default allows
  `clone`, `clone3` and `unshare` unconditionally in one large
  `SCMP_ACT_ALLOW` entry, and the allow won: in a box, `unshare -U true`
  succeeded and `unshare -Ur id` printed `uid=0`.
- **Fix (tested on this host).** Remove `clone`, `clone3` and `unshare` from
  every `SCMP_ACT_ALLOW` entry's `names`, then add for each of `clone` and
  `unshare` an `SCMP_ACT_ALLOW` rule when `(arg0 & CLONE_NEWUSER) == 0`
  (`SCMP_CMP_MASKED_EQ`, value `CLONE_NEWUSER`, valueTwo 0) and an
  `SCMP_ACT_ERRNO` errno 1 rule when `(arg0 & CLONE_NEWUSER) == CLONE_NEWUSER`.
  `clone3` then falls to the profile's default action, `ENOSYS`, and callers
  fall back to `clone`, where the flag is visible.
- **Observed after the fix.** `unshare -U` fails with "Operation not
  permitted"; fork, bun Workers, rtk and git all work.
