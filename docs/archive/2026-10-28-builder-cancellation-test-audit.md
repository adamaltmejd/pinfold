# Builder RUN cancellation regression audit

Date: 2026-10-28. Test-audit approval review for an extension of existing
guarantee 44, `linux_build_steps_stay_inside_the_builder_role`. No new test
function or production seam is added. The edited test has not been run.

1. **Guarantee row.** Row 44 now covers SIGINT during an audited RUN:
   exit 1, one failed JSON line containing the existing `cancelled` token,
   a dead VMM and unchanged public references. The same image name and
   private cache then run the existing normal successful build. Its role,
   allowed/denied egress and builder-absent launch checks are preserved.
2. **Named sabotage.** The comment names removing `JobReader::error`'s
   cancellation mapping. `read_exact` would retry semantic Interrupted
   indefinitely instead of returning a failed result; the bounded stdout
   read then times out. Skipping teardown or publishing interrupted output
   fails the pidfd or unchanged-ref assertion. No sabotage was executed.
3. **Existing gap and hardest case.** The earlier successful build could
   not exercise a cancellation while the host was reading a guest frame.
   The new real recipe requests the allowed API, then opens a FIFO with no
   writer. The proxy's allowed record establishes that RUN has started;
   the FIFO prevents normal completion after curl. The test signals the
   real owner while its VMM exists and before the build can publish.
   The ordinary build immediately afterward is the same-cache recovery
   control. This is RUN cancellation, not a late-cancellation ordering
   proof at the catalog commit point.
4. **Outside expected values.** The signal is Linux SIGINT. Exit and failed
   event expectations, the `cancelled` token and unchanged refs come from
   row 44 and the existing build contract. Readiness uses the production
   proxy's structured `host=api.github.com`, `decision=allowed` audit record,
   not stderr wording. VMM death comes from the kernel pidfd opened before
   signalling. Expected prior references are a host snapshot of only the
   private catalog's public reference map; comparing it afterward tests
   preservation, not an internal image layout. Existing role constants and
   fixture output bytes retain their previous outside sources.

The observer watches the private run root before the owner is spawned and
records its existing directories. It watches each new run directory before
looking for its egress file, then watches the file before reading existing
records. Only complete newline-terminated JSONL records are parsed. This
ordering covers creation and append events that precede the next poll.
Earlier run logs are excluded, and queue overflow fails the observation.

Inotify and owner stdout are polled together against one absolute 300-second
readiness deadline. Unexpected owner output/EOF fails readiness. The helper
does not sleep or retry a build. Log reads reuse the harness's 1 MiB bound.
After SIGINT, existing `read_bounded` reads stdout to EOF within 60 seconds;
the VMM pidfd must be readable within a further 10 seconds. These are
failure bounds, not performance measurements.

The real CLI is wrapped immediately in the existing ChildOwner. On panic or
output timeout it kills and waits for the CLI using existing bounded cleanup;
the production owner-lifetime mechanism ends its VMM. No reader thread,
fake guest, runtime stub, injected RPC response or binary flag is introduced.

The only dependency change enables the existing Nix `inotify` feature in
the e2e crate. It adds no crate or version and edits no lockfile. Nix's
installed wrapper supplies notification decoding; the existing pidfd pattern
and child/output helpers are reused. One local command constructor serves
the interrupted and successful calls with the same explicit API allowance.

The private readiness helper is 87 lines at `box_.rs:2016-2102`. The extended
G44 is at `:1816-2013`; the binary correction it exercises is the existing
`JobReader::error` adapter in `core/image/linux/job.rs`, which maps semantic
Interrupted to ConnectionAborted while preserving its cancellation detail.
This test revision does not edit that adapter or any production file.

The cost is one additional blocked real build before G44's existing build,
plus the inotify observer and host reference snapshot. It is a required
hard case for the reproduced cancellation hang. No final suite-budget pass
or runtime result is claimed. The parent session owns the manual proof,
final Linux execution, Mac checks and budget measurement. Targeted Rust
formatting passed; no build, test, VM, git or commit operation ran here.
