# Linux suite budget and remaining build cost

Date: 2026-10-29. [Ubuntu run 40890187071](https://github.com/adamaltmejd/pinfold/actions/runs/40890187071),
job `123504550063`, tested head
`7a4bf2dc724e27d7273a39f361c9f7c23341cf02`. Its synthetic merge commit was
`a22d30a386b3a6c6bbc8d7e8126b426e240857b5`. Both commits have tree
`98fad61eb43914523cf11562da448139a7e0ff7b`.

Formatting, locked all-target Clippy and AppArmor setup passed. The suite
passed **26 tests, 0 failed, 1 ignored**, in **684.30 s**. The wrapper
recorded `e2e_exit=0` and **688,636,229,738 ns**. CI correctly failed the
unchanged 300-second budget: the excess was **388.636 s**. The bootstrap
workflow passed. Functional success does not admit the runtime gate.

The previous complete run took 729.451 s. The difference is descriptive;
these CI runs are not a matched experiment establishing a causal saving.
No production source, limit, assertion or budget changed between them.

## Critical path from the raw CI log

These are wall-clock windows between observable test events, not isolated
test costs. Libtest includes time waiting for a runtime guard. Its
sixty-second notices do not identify active work or guard acquisition.

| Window, UTC | Duration | Observation |
|---|---:|---|
| 14:19:27.276 to 14:23:02.764 | 215.488 s | Tests begin through G23 completion; includes shared cold setup and the exclusive caller-build fixture. |
| 14:23:02.764 to 14:27:54.780 | 292.016 s | Remaining shared work and exclusive G17/G12 fixtures; private-cache G44 finishes at 14:25:29.726. |
| 14:27:54.780 to 14:30:28.569 | 153.789 s | Last shared test completion through exclusive G15 cleanup completion. |
| 14:30:28.569 to 14:30:51.577 | 23.008 s | Remaining exclusive runtime tests finish. |

The rounded windows sum to the 684.30-second test duration. Wrapper and
test startup/finish account for about 4.34 s more. The first and cleanup
windows alone total about 369.28 s, but cannot be treated as isolated
fixture costs or a proved lower bound for another machine or design.

## Static image-build accounting

The source counts below describe the normal Linux paths at this head.
They exclude trust/path refusals before image work and ordinary box
launches. Shared setup is listed separately to avoid charging it twice.

| Guarantee/fixture | Case CLI builds | Expected results |
|---|---:|---|
| G23 caller-owned builds | 13 | 12 successes, one intentional RUN failure. Four freshness-helper builds, three marker builds, an ONBUILD child, failure, identical repeat, explicit no-cache build and two profile reruns. |
| G15 cleanup | 11 | All succeed. Four changed-profile builds, two project builds, three retired images, one similar name and one identical twin. |
| G17 pre-creation refusal | 1 | Successful stale-tag setup; the later recipe-less launch refuses before image work. |
| G12 changed project | 1 | Successful final approved project build. Earlier build attempts refuse at trust/path/image-source boundaries. |
| G26 login route | 0 | Selects the existing default image. |
| G44 private builder/cache | 4 | Interrupted RUN, successful image, cache exhaustion, successful recovery. |
| Default-image setup | 3 across the suite's caches | One shared-cache default profile and separate defaults in G44's and G27 doctor's private caches. |

Source basis: G23 in `crates/e2e/tests/e2e/box_.rs`, its four-call
`selected_profile_freshness` helper in `pi.rs`, G15 in `runtime.rs`, and
the per-cache default-image mutex/set in `crates/e2e/src/lib.rs`.
Including those private-cache setups, these paths schedule 33 image-work
CLI calls, with 30 expected successes
and three expected failures. They are source counts, not CI process counts.

A normal case build runs a discovery VM and a build VM. A successful new
result can add a converter VM; an already completed root skips conversion.
G23 therefore has a conservative expected-path range of 26–38 case VM
lifetimes and G15 has 22–33, excluding setup. Both deliberately repeat an
identical image, which should reuse a retained completed root. Actual
conversion counts and per-phase times were not observed in this CI run.
Apple-only held/concurrent builds do not execute on Linux.

The remaining builds serve distinct freshness, reference, cache, failure,
retention, lease, sibling-image and shared-ID assertions. The independent
source review found no credible fixture-only path from 688.64 s to 300 s
that retains those observations. This is a review conclusion, not a proof
that all possible implementations must exceed the budget.

## Next bounded measurement and decision

The largest directly measured untraced removable transaction was G26's
31.584-second derived-profile build; it has already been removed. Normal
warm tiny builds still took 15.546–16.080 s in debug. Their host/VM split
is unknown without tracing. Release saved about 1.3 s per tiny build, and
the measured guest sync calls took milliseconds. Neither result explains
or closes the remaining gap. Ptrace-derived phase times include substantial
observer overhead and cannot supply the missing untraced attribution.

The next useful experiment is one normal invocation each of G23 and G15
on an owned Linux VM, using a low-overhead external kernel process
observer rather than ptrace. Record CLI and Firecracker exec/exit times,
the cold setup separately, actual converter counts, host gaps and the
interval from the last VM exit through CLI completion. Preserve raw events
and matching before/after executable
hashes. Assign phase names by the existing sequential production call
order. Do not add binary instrumentation, flags, weaker assertions or
another unchanged full-suite run. This note records the proposal only;
no such experiment was executed for it.

If the measured cost is repeated host preparation or verified immutable
artifact work, propose a bounded reuse change that retains validation and
publication controls, then measure that specific change. If finite VM
startup and separate discovery/build/conversion dominate, meeting 300 s
needs an explicit architecture decision: retain the current finite-role
lifecycle and leave admission blocked, or review a lifecycle redesign
that reduces starts while re-establishing every isolation, ownership,
cancellation and output-validation guarantee. Shared writable role reuse,
weaker isolation and a larger budget are not approved implementation
shortcuts. No architectural change or numeric-policy approval follows
from this accounting.

## Receipt and disposition

The complete raw log is preserved at
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/linux-pr252-7a4bf2d.log`,
SHA-256 `83706d736fe84bded7a1b30a0a0146fb4b28cd4139f6603f4b8071fb419f63f8`.
The source tree identity and local log digest were checked after completion.
Earlier phase measurements and their observation limits are recorded in
`2026-10-29-builder-phase-costs.md`.

The draft PR remains blocked on performance, numeric policy and the
previously recorded admission gaps. No new suite run, VM acquisition,
resource change, runtime merge or CLI release was performed for this note.
