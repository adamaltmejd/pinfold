# Observer total-buffer allocation, 2026-10-30

Diagnostic run 41046194129, job 124030763889, used source
e12d5960876a0ea22dca081936cfff63997ced89. It refused as config-buffer before
starting the suite. Its owned instance was stopped, disabled and removed.
No raw trace or public timeline was produced. The receipt records neither CPU
count nor reported total, so the exact failed condition remains unknown.

[Kernel tracing documentation](https://docs.kernel.org/trace/ftrace.html#the-file-system)
defines buffer_size_kb as a per-CPU request and buffer_total_size_kb as the
combined reported size. Allocations use native pages and subbuffers and may
include extra metadata. The previous 1024 KiB request per CPU did not divide
the existing 4096 KiB total allowance across the instance's CPU set. This is a
source defect, not proof of the failed runner's topology or allocation.

This repair freezes the stopped instance's existing canonical CPU directories.
The count must be positive and bounded. It validates the host page size and
requires the documented per-instance subbuffer-size readback to be a positive
power-of-two multiple of that page size. Missing or unsupported geometry refuses;
there is no guessed unit or fallback sequence, and the subbuffer setting is
never written.

For C CPUs and native unit U KiB, the single per-CPU request is
(floor(4096 / C / U) - 1) * U. It must be positive before writing. This leaves
one unit per CPU for rounding. It does not predict the actual allocation:
positive total readback at most 4096 KiB, positive bounded per-CPU readbacks,
unchanged CPU IDs and unchanged native unit remain authoritative. There is one
size write, no adaptive resizing and no cap increase. Reported tracing KiB is
not a physical-memory guarantee including metadata.

The new _cpu_ids field travels through the private helper exchange and is removed
before public receipt storage. Existing per-CPU statistics and their numeric keys
are unchanged. Each start checks the frozen set before enabling tracing; existing
loss-counter set checks cover the captures.
The only new public sizing diagnostics are bounded integers named cpu_count,
requested_per_cpu_kib and reported_total_kib. Failures report only available
validated values, fixed refusal stages and optional numeric errno.

The workflow, budget command, native schemas, framing, privacy/publication
gates, loss checks, readiness, cleanup and cancellation scope are unchanged.
Runtime, tests, pins, product caps and the 300-second budget are unchanged.
Only source review, AST syntax parsing and whitespace checks run here. No
observer import, probe, readiness check, test, build, VM, push or CI execution
is authorized by this source repair. Parent exact-source reviews precede any
further execution decision; no compatibility or timing success is claimed.
