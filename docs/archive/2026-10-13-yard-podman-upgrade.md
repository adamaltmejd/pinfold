# Yard stock Podman 6.1.4 upgrade

The operator requested the upgrade despite the unresolved contention and
LXC exec-cleanup findings in
`2026-10-13-stock-podman-contention-comparison.md`.

Yard now resolves `podman` to `/usr/local/bin/podman`, a symlink to the
root-owned `/usr/local/lib/podman-6.1.4/podman`. This is the unchanged
upstream build from commit `464c15fec06f0074c916f9eff87c4806869c2833`,
SHA-256 `11326740634a0929f438256c4e3ff90273c4399a279c68e9fde02be25f75c2a5`.
The distribution's `/usr/bin/podman` remains installed at 5.4.2. This is
an operator-installed source build, not a Debian package upgrade or an
automatic update mechanism.

The unchanged upstream storage template was installed as
`/home/yard/.config/containers/storage.conf`, mode 0600, SHA-256
`322dffd5a543bb9e1ca9c67cc56999c62b746118674d004180fe60dddfc21979`.
It overrides the older vendor template for Yard only. Both the old runtime
with this template and the new runtime reported the existing paths:

- Graph root: `/home/yard/.local/share/containers/storage`.
- Run root: `/run/user/1000/containers`.

The store already used SQLite. A consistent SQLite backup was saved before
the switch. No `system migrate`, storage reset, FUSE change, security
relaxation, service restart or forced container stop was performed.

## Validation

The existing live box retained the same ID and owner immediately after
the switch. Its Yard execution 2761 later ended normally with outcome
`candidate` and no error detail; Yard advanced to its checks. The daemon's
PID, start ticks and executable identity stayed unchanged, as did the
installed Pinfold binary.

A uniquely named smoke box used the installed Pinfold, active runtime
store and cached default image. Pinfold state/config/cache were isolated
in a temporary directory, with unrelated maintenance excluded. Startup,
exec output, stdin-triggered teardown and absence from the final native
inventory all passed. Its temporary Pinfold files were removed.
`pinfold doctor` reported Podman 6.1.4, successful runtime preflight and
enabled linger. This smoke is not a repeat of the contention workload and
does not resolve either known failure.

Evidence, configuration/build identities, before/after inventories, SQLite
backup, smoke output and final result are retained at:
`/home/yard/validation/pinfold-89-lxc-20261012/stock-version-20261013/host-upgrade/`.

## Recommended next investigation

Keep Pinfold's bounded inventory failure. Reduce the storage failure to a
stock-Podman-only reproduction: a cold many-file image, a keep-id create,
a concurrent listing, actual lock-holder observations, a warm repeat and
a newly built derived image. Verify writer success and ownership as well
as the blocked listing. This should support an upstream correction to
the ownership-conversion lock duration. Keep exec-PID-handle EPERM separate.

Moving Yard to a VM is not yet a demonstrated remedy: the earlier stock
6.1.4 VM smoke also reported `Supports shifting: false`. Exact-image
preparation is a limited mitigation; new and rebuilt images can repeat
the conversion. No additional Pinfold image-management layer, runtime fork
or infrastructure migration was introduced, and no upstream report was
published during this upgrade.
