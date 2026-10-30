# Builder-role base fixture and test audit, 2026-10-30

This consolidation starts from
`c6cc1bea1de4ad6e97b299bab9d079b568674d4a`. Its Ubuntu suite passed
26 tests, failed none and ignored one. Body time was 349.32 seconds; wrapper
time was 353.927805619 seconds. The earlier d62eea9 run had the same functional
counts, a 412.67-second body and a 415.967693984-second wrapper. Both exceeded
the unchanged 300-second budget. Both bootstrap jobs passed. These whole-suite
observations include scheduling and contention; they are not isolated fixture
measurements or validation of this change.

Guarantee 44's initial image preparation no longer builds the default profile.
It builds one real caller image from the same debian:trixie-slim through the
existing CLI helper, in the same private cache. The recipe explicitly installs
ca-certificates, coreutils, curl, dash, mawk and sed without recommendations.
They supply HTTPS trust, /bin/sh, id, awk, cat, sed, printf and mkfifo for the
existing guest commands. The static cache-fill program is still compiled on
the host. The completed box still executes cat to read the observed fixture.

The removed setup installed unrelated Git, ripgrep, fd-find, jq and less,
configured Git, upgraded packages and stripped setuid/setgid bits. None is an
expected value or prerequisite of this row's assertions. The built-in default
contains no Bun; no Bun cost is attributed to it. Built-in profiles and their
hardening remain unchanged. Security remains in the binary.

The base name is distinct from the tested output name, and preparation finishes
before reference snapshots and run watches. Its separate context leaves the
four case recipes and their context unchanged. It uses the already allowed
deb.debian.org host; no --allow addition, global policy or tested case permission
changes. The actual role-case build still adds only api.github.com. No shared
cache or image-cleanup guard is introduced. There is still one initial base
build and the same four case builds.

## Candidate audit

- Row: 44, Linux image work stays in its own VM. No new test or guarantee.
- Failure proof: the retained sabotage comments name host execution, role
  controls, NIC exposure, proxy policy/audit bypass, cancellation mapping,
  teardown/publication, failed-cache pruning and launch-time bootstrapping.
- Existing blocks remain the proof. SIGINT during an audited RUN, real ENOSPC,
  same-cache recovery and completed-image launch with the builder unavailable
  remain the hard cases. No assertion block is added, cut or rewritten.
- Expected values still come from the approved role, guest kernel, HTTP status,
  fixture bytes and host-observed refs, run logs and VMM lifetime. Every fault
  observer and positive control is preserved.

The source change is confined to G44's initial base preparation: 23 test lines
added and one removed; zero binary lines change. All four case commands,
assertions, private-cache use, SIGINT/ENOSPC recovery, clean, builder obstruction,
offline completed-image launch, inotify and reference observers are unchanged.
The five G15/G23 cases, G12, G17 and G27 are unchanged. No artifact, pin,
converter derivation input, binary dependency, cap, resource policy, protocol
or budget changes. The guarantees table needs no edit.

The worker performs source review only, with no formatting, compilation, tests,
VM, push or publication. The operator owns compilation, real Ubuntu G44 and
full-suite budget validation. No new pass, isolated saving or budget success
is claimed. Runtime merge, a CLI release and numeric approval remain outside
this change.
