# Guarantees table cut (2026-11-20, #297 first step)

The Guarantees table in ARCHITECTURE.md went from 31 rows to 15. Each
surviving row is one test: a short statement, one hardest scenario and its
positive control. The source was a draft cut list written for the owner
before the suite rewrite. Its method: keep anything that defends a
threat-model control or the no-secret rule, keep one caller contract per
family, drop wording, layout and Git's or the runtime's own behaviour, and
drop twins.

A cut removes a proof, not a control. No control in the threat model,
Always applied or the proxy rules changed.

## Owner decisions

- Kept: 1, 2, 5, 8, 10, 11, 15, 26, 38, 43, 44, 45, 47, 48, 50.
- Cut with no replacement check: 9 and 17 (no program but pinfold calls
  `pinfold box`), 28 (updates), 49 (Rex relay), 34 and 36 (slow proxy
  timeouts), 20 and 27.
- No private-address (10/8, 192.168/16) scenario. Row 2 keeps `localhost`
  and the host's own address.
- Row 10 stays broad: the owner uses disk projects and `git fetch box` on
  macOS. Parser edges are dropped.
- Row 44 is one Linux build with a denied and an allowed host.
- Row 26 uses Pi and Claude through the fixture. The live Codex login
  leaves the suite, so the only public endpoints are `api.github.com` and
  `example.com`.
- TTY detail scenarios are dropped.

## Rows

Before (31): 1, 2, 4, 5, 6, 8, 9, 10, 11, 13, 15, 17, 18, 20, 21, 23, 26,
27, 28, 34, 36, 38, 39, 42, 43, 44, 45, 47, 48, 49, 50.

After (15): 1, 2, 5, 8, 10, 11, 15, 26, 38, 43, 44, 45, 47, 48, 50.
Numbers are kept; gaps are cut rows.

| # | Fate |
|---|---|
| 1 | Kept. Dropped the `/proc/config.gz` check (how the kernel was built). Added the Linux check that without `egress` CID 2 port 3128 is reset too. |
| 2 | Shrunk. Absorbed 4's routing. Dropped ambiguous framing, the raw POST, CONNECT to a route and `ready` naming the log. |
| 4 | Merged: routing into 2, audit-log stop into 38. |
| 5 | Shrunk to the fork+exec grandchild (`unshare -U` fails with `ENOSPC`), the setuid inventory of the built-in `default` image and the read-only root. The kernel check is two boxes' distinct `boot_id`, since a differing `uname -r` does not prove a separate kernel. |
| 6 | Secret part merged into 26; the rest is prose (Process interface, Always applied). |
| 8 | Shrunk to SIGKILL of `box up` and prune, plus a detached owner's client exiting 125. On Apple, `box exec` shows the request gets no upstream response inside the surviving guest, so the failure is the proxy being gone, not a dead exec. The guest's loopback relay may still accept the connection. |
| 9 | Cut. Lifecycle contract stays in Process interface prose. |
| 10 | Shrunk to two fetch steps (a listing naming a host-only commit fails; a valid tag colliding with a host tag leaves host tags in place under force and prune), planted hooks, a `push.followTags` push to `host/*`, fresh disk and first push (from 13). The statement covers only the host's own branches and tags, with the exceptions the threat model lists. Parser edges, quarantine races, promisor and wording dropped. One test with several steps. |
| 11 | Shrunk to the writable race, `disk-in-use` then `disk-unclean` after owner death (the dead claim pins the disk until `box prune` retires it, per `core/disk.rs`), and `disk rm` in use. |
| 13 | Protected paths into 48; disk first start and first push into 10; the rest is prose. |
| 15 | Shrunk to one `clean --unused 0s` beside a kept live project. |
| 17 | Cut. Validation stays in Process interface prose. |
| 18 | No-config refusal into 47; the rest is Configuration prose. |
| 20 | Cut. `stat` and OOM scoring stay in prose. |
| 21 | Merged into 26. |
| 23 | Managed-build identity into 50; caller-build detail is Images prose. Its apt sentence, which contradicted Images, is gone. |
| 26 | Shrunk. Absorbed 21 and the secret parts of 6 and 39. One injecting route (the box's own header value replaced) and Claude and Pi login routes (duplicates and nominated fields removed, `?api_key=` refused) at a fixture that echoes nothing. Control includes the `from` and `PINFOLD_ENV_` values arriving. The statement keeps the threat model's reflecting-upstream exception. Live Codex dropped. |
| 27 | Cut. `doctor` stays in Maintenance and CLI prose; its protected-path finding goes with 48. |
| 28 | Cut. Updates prose unchanged. |
| 34 | Cut. Blocked writes stay in Egress proxy Limits. `slow-response.yml` deleted. |
| 36 | Cut. Idle tunnels stay in Egress proxy Limits. `slow-proxy.yml` deleted. |
| 38 | Shrunk to the audit-log cap stop. Absorbed 4's audit stop, 42's cap stop and 39's launch record. Run-record retention (14-day age, the 128 MiB inactive quota, a live run left untouched) now has no test, by the owner's choice of a short table; it stays in Maintenance prose. |
| 39 | Split: secrets into 26, launch record into 38, command contract and fixed policy into 47. TTY detail and the "no harness version field" absence check dropped. |
| 42 | Merged into 38. |
| 43 | Kept, without the fds check (moved to prose). |
| 44 | Shrunk to one build with deny and allow, plus a direct connection from RUN that bypasses the proxy and fails (no NIC), which tells the build VM from a host build behind the proxy. |
| 45 | Shrunk to symlink, executability (export normalizes modes to 0644 or 0755) and existing destination. Retitled "Disk export writes only a new destination": `box exec` output also leaves a disk, so export is not the only way. |
| 47 | Shrunk. Absorbed 39's command contract and fixed policy and 18's no-config refusal. One test with several steps. |
| 48 | Shrunk. Absorbed 13's protected paths, as two separate refusals (configuration directory, profile directory) so neither masks the other. The statement names the hard-link exception, and the title no longer claims no project reaches pinfold's configuration. The hard-link scenario, which asserted a listed non-protection, is gone. |
| 49 | Cut. The pi layer's Rex prose gained two sentences (below): concurrent starts share one server and socket, and owner death ends every dial exec. |
| 50 | Shrunk to a three-link chain. Absorbed 23's identity. The "`full` is an unknown profile" absence check dropped. |

## Sentences moved to prose

These behaviours were stated only in a dropped scenario:

- Process interface: "`down` of an absent, empty or flag-like name other
  than `--help` and `-h`, which print help, exits 0, prints nothing and
  leaves any live box alone." (from 9)
- Always applied, closing paragraph: "On Linux box work cannot list the
  exec service's `/proc/<pid>/fd`." (from 5)
- VMM jail: "The VMM's standard descriptors are `/dev/null`." (from 43)
- Process supervision: "A real terminal exec preserves the caller's input
  file status flags, blocking or nonblocking." (from 9; prose named only
  Apple)
- Egress proxy, Limits: an idle tunnel's "log line names `idle timeout`."
  (from 36)
- pi layer, Arguments: "An execute-only directory on that walk works."
  (from 39)
- pi layer, Lifetime: "Bootstrap death or a final departure racing a join,
  keep or stop never admits two writable generations." (from 47)

A review of the first commit found more behaviour stated only in dropped
scenarios. Each was checked against the code or a dated record:

- Process interface, `env`: a literal `HOME` appears in "Apple's container
  configuration", and "a refusal of such a mount may name it." (from 6)
- Configuration, `config --json`: "On Linux it reads only the local
  catalog, so it needs neither KVM nor a runtime on PATH, and an absent
  catalog reports false." (from 18; `catalog::resolve`)
- Process interface, `stat`: "A command the guest kernel OOM-kills exits
  137 itself; on Linux `oom_kills` then rises." (from 20; Apple reports
  `oom_kills` null. The old row's 64 MiB `MemTotal` margin was observed
  for one 256 MiB box and is dropped.)
- Always applied, Apple: PID 1 "sets `no_new_privs` on itself first;
  failing to set it fails startup." (from 5; `init.rs` calls
  `set_no_new_privs` before the user-namespace check)
- Process supervision, OOM score: "-1000 and legacy -17 fail with `EACCES`
  and leave the score unchanged, while legacy 15 succeeds." (from 20; the
  exec service sets the -999 floor)
- Process supervision, TTY: "Later host resizes reach the guest terminal
  while the command runs." (from 39)
- pi layer, Rex: "Concurrent starts share one server exec and one socket."
  and owner death ends "every dial exec". (from 49; observed in
  2026-11-19-rex-relay.md)

## Workflows and policy

- Deleted `.github/workflows/slow-response.yml` (row 34's gate) and
  `slow-proxy.yml` (row 36's). Each held only its gate: checkout, Linux
  runner setup, musl build, the AppArmor profile, and the one test or
  `scripts/slow-proxy.sh`.
- `release.yml` no longer needs `slow-response`. That gate was the only
  test in the release workflow, so a pushed tag now publishes. The nightly
  pin release still runs the suite and fails without it.
- AGENTS.md Tests: guarantee 36's namespace sentences and the Codex live
  login are gone from the public-endpoints rule. `example.com` is denied
  by default and allowed only as guarantee 44's control. A positive
  control succeeds in the same box. Where the refusal comes from the box's
  or build's own configuration, or before any box exists, it succeeds in a
  second one in the same project that differs only in that setting.
- Deleted `scripts/slow-proxy.sh`, `scripts/connect_tunnels_share_activity.py`
  and `scripts/connect-tls.Containerfile`. They served only row 36, and
  nothing else used them.
- `bump-pins.yml`: the Mac suite step is still named "Full Mac end-to-end
  suite, including live login". Renaming it was refused at push: the
  session's GitHub token lacks the `workflow` scope for that file.

## Size

ARCHITECTURE.md: 2270 lines before, 2266 after.
