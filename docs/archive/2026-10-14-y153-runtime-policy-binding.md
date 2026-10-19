# Y153 seccomp policy binding investigation

2026-10-14. Investigation on yard-sthlm using installed pinfold 0.2.2,
Podman 6.1.4 and crun 1.21. No implementation, installation, release or
Yard board change.

Two findings:

- Concurrent pinfold launches with different host seccomp profiles can
  consume the same shared cached profile. This was reproduced through the
  real CLI. The profiles used here both retained the namespace block.
- crun has a per-container BPF record that normal `exec` actually reads.
  A version-specific host observer can bind that added filter to a live
  execution. Podman's OCI JSON alone cannot do that. This does not identify
  inherited filters or settle all of Yard's runtime inputs.

Y153, Y150 and Y63 remain unresolved. The evidence supports a narrower
observer investigation before proposing a new pinfold reporting API.

## Evidence and identity

The operator handoff and prerequisite reports were read from
`/home/yard/validation/y153-oci-policy-probe-20261014/` and
`/home/yard/validation/y111-operator-evidence-20261013/y153-621f66-prerequisite-hold/`.

New remote evidence is retained under
`/home/yard/validation/y153-policy-binding-20261014-codex/`.
Local copies:

- [Policy controls](2026-10-14-y153-runtime-policy-binding/policy-controls.json)
- [Held exec](2026-10-14-y153-runtime-policy-binding/held-exec.json)
- [Policy probe](2026-10-14-y153-runtime-policy-binding/policy-probe.py)
- [Held exec probe](2026-10-14-y153-runtime-policy-binding/held-exec-probe.py)

The completed policy run is remote `attempt4`; the held execution is
`held-exec`. Earlier attempts retain fixture failures: an unsupported file
mount, an incorrect assumption that successful `getppid` must be positive,
and a duplicate evidence filename. They are not product failures. Each
attempt left its disposable containers absent. `getppid` legitimately
returns zero when the parent is outside the guest PID namespace.

The installed pinfold SHA-256 is
`7d1a82ad1496fc855ff19c706f3a41e83419c7ba500bd6288ba0d63c0747a27f`.
The policy-control report records the Podman and crun executable hashes.
All three executable hashes were unchanged after that run. The held exec
also hashed the actual running host `pinfold box exec` executable through
`/proc/<pid>/exe`, with the same result.

Source checkout identities used for the runtime trace:

- Podman 6.1.4: `464c15fec06f0074c916f9eff87c4806869c2833`.
- crun 1.21: `10269840aa07fb7e6b7e1acff6198692d8ff5c88`.
- Local pinfold HEAD: `198600b1751d889d354ed3657d8f839e8436e469`.

The installed crun reports that source commit and Debian package
`crun_1.21-1_amd64`; `dpkg --verify crun` reported no mismatch. These are
artifact and source identities, not a reproducible-build proof linking
every installed binary byte to those sources. The local dirty Podman
adapter changes do not alter the shared seccomp derivation function.

Only policy JSON, BPF bytes, selected process-status fields and fixture
results were retained remotely. Whole OCI configuration, environments and
credential values were not retained or fingerprinted. Probe cache/state
roots and process-local host profiles were separate from the daemon's.
The probes used an existing local image, no egress and no credentials.

## Reproduced policy selection race

`Podman::up` derives the host profile, atomically replaces the shared
`cache/seccomp/podman.json`, then starts Podman with that pathname.
The rename prevents partially written JSON. It does not reserve those
bytes for the launch that wrote them.

The probe used two copies of the host profile. The second additionally
denied `getppid` with `EPERM`. Pinfold still applied its normal namespace
restrictions to both. Both launch processes shared only the probe cache.
No wrapper, runtime stub, sleep or timing retry was used.

| Scenario | `getppid` outcome | crun BPF digest prefix |
|---|---|---|
| Normal profile | Allowed, return 0, errno 0 | `4e2fdd050f4f` |
| Stricter profile, sequential launch | Refused, return -1, errno 1 | `c8bc43e7184e` |
| Existing normal box after host profile change | Still allowed | `4e2fdd050f4f` |
| Same name after teardown and replacement | Refused; new container and generation | `c8bc43e7184e` |
| Concurrent normal launch | Allowed | `4e2fdd050f4f` |
| Concurrent stricter launch | Unexpectedly allowed | `4e2fdd050f4f` |

Both concurrent containers' OCI policy and BPF matched the normal profile,
and their real syscall outcomes agreed. This is a consumed-input mismatch,
not merely a post-launch cache hash mismatch. It does not show a containment
escape: every observation still returned `EPERM` for `unshare(CLONE_NEWUSER)`
and ran as uid 1000.

The smallest prospective pinfold correction is to write the derived
profile into the claimed box state directory and pass that unique pathname
to Podman. Core already has that directory, including for boxes without
egress. Its owner already retains and cleans the state through runtime
removal. Avoid deriving a state path from the optional proxy socket or
adding a global launch lock. The caller-readable identity question is
separate from this startup selection fix.

The current guarantees table does not explicitly promise independent
selection of changing host profiles. Before implementation, specify that
guarantee and its hardest real concurrent CLI scenario. Preserve all
existing controls, owner exit status and teardown.

## Record consumed by crun exec

On create, crun generates or retrieves BPF into its state directory and
installs it through an open descriptor. On normal exec, it loads its own
saved `config.json`, opens its existing `seccomp.bpf` read-only, and applies
those BPF bytes with the saved installation flags. It does not regenerate
BPF from the syscall rules in Podman's `OCIConfigPath`.

For this host the record was
`/run/user/1000/crun/<full-container-id>/seccomp.bpf`. State root is a
runtime selection, not a universal pathname. An explicit crun `--root`
can change it.

Normal exec does not modify that record. Other containers' cache hits skip
BPF generation; cache insertion links a completed new file; cache eviction
unlinks cache entries and avoids files with multiple links. A host profile
change affects future creation. crun resource updates change cgroups, not
the saved BPF or installation configuration. Arbitrary same-user file
mutation is outside the stated trusted-host subset; this record is not an
immutable object against such mutation.

Material source conditions:

- A missing BPF file makes crun's exec opener return an absent descriptor;
  the application function then skips installation. OCI JSON can still
  describe seccomp. Missing or unreadable BPF must remain unknown.
- Preserve absent versus present installation flags. With no flags crun
  selects its default, and an unsupported-flags `EINVAL` can cause a retry
  with zero flags. Runtime artifact and kernel context matter.
- Notification behavior can come from saved listener fields, receiver or
  plugin annotations, or host `RUN_OCI_SECCOMP_RECEIVER`. Exclude it or bind
  its behavior for each invocation. A safe presence boolean suffices for
  rejecting it; no environment values need to be logged.
- Creation hooks and any other trusted host actor that can alter this
  state must be accounted for. Empty OCI hooks do not by themselves prove
  that Podman's precreate hook configuration was empty.

The probe-specific configuration supplied only a seccomp path. The default
held-exec host configuration snapshot contained the vendor containers.conf,
no configured `hooks_dir`, and no engine environment entries. Rootless
Podman's default has no implicit hook processing. The saved records had no
notification settings; the held exec's allowlisted host environment omitted
`RUN_OCI_SECCOMP_RECEIVER`. These snapshots do not establish another
caller's launch configuration.

## Binding to a real execution

The held probe ran a fixed static observer from a read-only mount through
`pinfold box exec`. It reported readiness and waited for a host input signal.
Before releasing it, the host selected the live Podman exec session, read
its `exec_pid`, and checked:

- Its host `/proc/<pid>/exe` hash equalled the mounted observer's bytes.
- Its `NSpid` matched the guest-reported PID.
- The box's full runtime ID, generation, init PID and start time identified
  the live owner lifetime.
- The real observer had `Seccomp=2`, `NoNewPrivs=1`, zero effective and
  bounding capabilities, and two installed seccomp filters.
- The saved BPF and selected safe installation fields belonged to that
  container's crun state directory.

The host then released the observer. Its deliberate exit 42 reached the
caller unchanged. Container lifetime and BPF remained unchanged afterward.
The observer executable hash is
`4b5f3450d5204b3a64a6ae66cf2cf3d9ca339b82c0646e6496ce47f6ac8ffdad`.
This exercises an existing host observation path. It is not a Yard gate
integration or an observation of a production candidate's execution.

## Precise remaining boundary

The launch host is LXC on kernel `7.0.14-19-pve`. A host process already
reported one inherited seccomp filter; the real guest exec reported two.
The saved crun BPF identifies the added filter, not the whole effective
filter stack. Inherited bytes remain unknown.

Linux inherits filters across fork and exec and permits additional
filters. A conservative observer could bind inherited-stack equality to
one measured common launching ancestry interval. It must observe the
actual launching thread's identity and baseline before the first compared
gate, preserve that identity and count through reuse, and validate the
actual descendants and added-filter count. Daemon PID alone is insufficient:
threads can add or synchronize filters without replacing the process.
Inherited notification behavior also needs a trusted exclusion or binding.
The present probes did not establish this interval across two Yard gates.

Therefore a supported observer for this exact Linux backend is a concrete
avenue; a new pinfold API has not been shown necessary. Its minimum record
would bind the actual execution, owner generation, full runtime container
and exec process identities, runtime artifacts, the saved BPF digest and
installation settings, and a qualified inherited-stack boundary. Missing
records, changed lifetime, unsupported backends, notifications or unproved
launch ancestry remain unknown. Historic executions without these
observations cannot become reusable retroactively.

This seccomp record cannot settle environment, resource, kernel, init or
tool identity by itself. Podman can update environment/resource settings
without changing the init PID or start time. Those are separate obligations
in Yard's existing runtime-input work.

## Teardown and scope

All six boxes in the two completed probes ended through owner stdin
closure, emitted `down`, returned owner exit 0, and were verified absent.
Same-name replacement accounts for two separate owners. The reports retain
the individual observations. No installed binary or daemon service was
changed. Only dated investigation evidence was added to this checkout.

## Primary sources

- [Podman inspect interface](https://docs.podman.io/en/stable/markdown/podman-container-inspect.1.html)
- [Podman creation and precreate hooks](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/libpod/container_internal.go#L1035-L1075)
- [Podman exec process selection](https://github.com/podman-container-tools/podman/blob/464c15fec06f0074c916f9eff87c4806869c2833/libpod/oci_conmon_exec_common.go#L681)
- [crun exec record selection](https://github.com/containers/crun/blob/10269840aa07fb7e6b7e1acff6198692d8ff5c88/src/libcrun/container.c#L3631-L3687)
- [crun BPF opening](https://github.com/containers/crun/blob/10269840aa07fb7e6b7e1acff6198692d8ff5c88/src/libcrun/seccomp.c#L860-L913)
- [crun BPF application and flags](https://github.com/containers/crun/blob/10269840aa07fb7e6b7e1acff6198692d8ff5c88/src/libcrun/seccomp.c#L200-L246)
- [crun cache insertion and use](https://github.com/containers/crun/blob/10269840aa07fb7e6b7e1acff6198692d8ff5c88/src/libcrun/seccomp.c#L529-L669)
- [crun notification settings](https://github.com/containers/crun/blob/10269840aa07fb7e6b7e1acff6198692d8ff5c88/src/libcrun/container.c#L2337-L2377)
- [Linux filter inheritance and composition](https://docs.kernel.org/userspace-api/seccomp_filter.html)
