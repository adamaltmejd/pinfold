# Admission-scan cost and unresolved host contract

2026-10-10. Follow-up to Y-9/Y-10, at repository head
`3aa58685363ccc9c95072991b06e3ad9082bebf9`. No production code or binding
contract changed. Linked worktrees remain refused.

## Metadata-only measurements

On macOS 27.0.1 arm64, a temporary Python script created plain, empty files
under `/private/tmp`, then scanned their metadata without reading contents.
Each case had three scans, each in a fresh Python process. Fixture creation
was outside the timed region. Fixtures were removed afterward.

The first traversal inspected reference entries and collected device/inode
identities only for multiply linked files. The second inspected all
writable entries and checked their identities against that set. Directory
symlinks were not followed. All fixtures had single-link files, so every
candidate set was empty. This measures full traversal cost only, not alias
classification, a complete mount graph, or a production implementation.

| Fixture | Reference files | Writable files | Metadata calls | Total seconds, min / median / max |
|---|---:|---:|---:|---:|
| Small | 64 | 1,000 | 1,069 | 0.0027 / 0.0028 / 0.0030 |
| Large ordinary tree | 64 | 100,000 | 100,465 | 0.2551 / 0.2644 / 0.2655 |
| Many reference entries | 20,000 | 100,000 | 120,480 | 0.3050 / 0.3153 / 0.3225 |

Peak RSS was 17.9-18.1 MiB including the Python interpreter. It does not
measure a large candidate identity set. Directory entries account for the
difference between file counts and metadata calls.

Caches were warm/uncontrolled: the files had just been created, and no OS
cache flush occurred. There were no real Git object stores, aliases,
overlapping exports, unreadable directories or concurrent changes. These
are synthetic observations, not loose/packed clone benchmarks or a bound
for large repositories. A real admission algorithm could skip work in
some layouts or require more work in others; no shortcut was validated.

The script and complete JSON are `/private/tmp/pinfold-metadata-cost.py`
and `/private/tmp/pinfold-metadata-cost.json`. The earlier Linux startup
measurements used another host and a cached minimal image without an agent;
they must not be added to these Mac scan times as a measured total.

Yard Y-11, using Sol medium, reviewed the supplied observations and source
mechanism. The operator accepted its conclusion: this is a narrow full-walk
cost observation, with no measured problem justifying an optimization and
no implementation child admitted. Y-11 was closed as completed assessment.
No test assertions changed and no end-to-end suite was rerun for this work.

## Independent Astra review

Astra medium reviewed the current spec, existing evidence and Y-9/Y-10
read-only. Its decision was not to admit the guard yet. The runtime
boundary remains incompletely validated, and the proposed startup-only
classification depends on a host responsibility absent from the spec.

The review distinguished trusted host/caller operations from concurrent
untrusted boxes. Per-name lifetime locks do not prevent another box from
altering overlapping writable host storage. Guest actions in another box
must not be treated as trusted host mutations to make the proposal pass.

## Proposed decision text, not an adopted contract

For consideration if the caller can enforce this ownership rule:

> Trusted host tools may perform ordinary Git operations and replace files
> within protected directories. From admission until teardown, the caller
> must not expose protected metadata through new writable aliases or change
> protected mount-source topology. This responsibility does not exempt
> actions by untrusted boxes. Every permitted concurrent-box arrangement
> must preserve the protection against actions by any of those boxes.

Accepting that responsibility would require a binding spec change and
runtime evidence that ordinary host updates stay supported. It does not
authorize implementation, establish alias-complete enforcement, or satisfy
the blocked Linux validation. Requiring protection even against host-created
writable aliases during a run rules out a startup-only scan.

Y-10 remains parked. No blocked probe was retried or delegated. The cost
observations are independently useful but cannot close the security gate.
The host-responsibility question was presented to the user; no answer or
contract approval is assumed in this record.
