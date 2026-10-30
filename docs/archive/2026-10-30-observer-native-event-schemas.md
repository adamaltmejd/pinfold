# Observer native event schemas, 2026-10-30

Diagnostic run 41043502023, job 124021781325, used observer source
1433a06233052168eeebc226067330598d53bf85. Create/config preflight failed.
The receipt reports suite_started false and successful stop, event disable
and removal of the owned instance. No trace or public timeline was produced.
The exact failed check is unknown; the old receipt carries no refusal stage.
This change does not establish its cause or justify another run.

[Linux v6.17 sched.h](https://raw.githubusercontent.com/torvalds/linux/v6.17/include/trace/events/sched.h)
declares fork task names as strings and exit with a fixed task-name array plus
a group_dead boolean. Its exit print format includes that boolean as the
literal true or false. The
[structure macros](https://raw.githubusercontent.com/torvalds/linux/v6.17/include/trace/stages/stage1_struct_define.h)
store strings through u32 data-location descriptors; the
[field macros](https://raw.githubusercontent.com/torvalds/linux/v6.17/include/trace/stages/stage4_event_fields.h)
declare their type as __data_loc char[] with size four and alignment four.
These are source findings, not the failed runner's observed schema.

The observer retains its fixed-array fork and exit shapes and unchanged exec
shape. It additionally accepts only the explicit v6.17 string-descriptor fork
and boolean-suffixed exit shapes. Each accepted variant matches the complete
non-common field declarations, offsets and sizes and its exact print format.
Unknown or duplicate fields refuse. Exit parsing accepts only the old suffix
or the exact group_dead=true/false suffix; the value adds no output or group
lifecycle inference. Arbitrary names remain redacted.

Setup, create and config refusals now report fixed allowlisted stage tokens
and optionally a bounded integer errno. Exception messages, paths and raw
formats are never emitted. Empty PID-filter files remain required. The
trace_events.c fetch timed out and was not retried; this cut changes no PID
filter semantics or text interpretation.

The native entry-count/LF/header framing, private bounded raw storage,
sanitized publication gate, loss checks, readiness, cleanup, cancellation
scope and normal-user budget command are unchanged. Workflow, runtime,
security controls, artifacts, pins, assertions and budget are unchanged.
Only source review, AST syntax parsing and whitespace checks run here. No
observer import, probe, test, build, VM, CI run or push is authorized by this
repair. Actual kernel compatibility remains unverified. Parent source reviews
must precede any execution decision; this is not budget admission.
