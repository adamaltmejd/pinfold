# Shared-cache test scheduling and Ubuntu timing

Date: 2026-10-29. Ubuntu run `40878605851`, job `123463792840`, checked out
synthetic merge `052c6d21fab2f2455e179b5e40897afac79f43b6`, merging runtime
head `30d16d99b38e060aebfa89cdab7d4652162c0d9d` into main `892e58d9`.
The checked-out tree and runtime head both identify
`b4eaac452f97f78cbca618cf507f2ab7a20dbdc2`.

Formatting, locked all-target Clippy and AppArmor setup passed. The suite
reported **24 passed, 2 failed, 1 ignored**, in 477.38 s. The budget wrapper
recorded 481,807,565,923 ns and exit 101. Neither the full runtime gate nor
the 300-second suite budget passed.

Both failures were competing ordinary builds of the suite's shared Linux
image cache. G26's profile build failed with `builder-in-use`; G23's later
uncached caller build returned the same refusal. The real post-commit EIO
receipt in G23 recorded one injected errno 5, child exit 0 and no supervisor
error before that later refusal. G44 passed in this full run.

The production cache intentionally has one transaction owner. Several test
fixtures instead held a shared runtime guard while building into the same
cache. G23 downgraded its exclusive guard after the first caller build even
though further builds followed. This allowed unrelated tests to race the
fixture's cache; production correctly refused the competing call.

## Bounded fixture change

Linux tests that build further images in the shared cache now use the
existing exclusive runtime guard for their complete fixture:

- G23, caller-owned image builds: retain the guard; delete the downgrade.
- G17, pre-creation refusal: protect its recipe-less profile setup build.
- G26, host login route: protect its own profile build.
- G12, changed project file: protect its final approved project build.

Other shared tests only use the existing serialized default-image setup.
Maintenance tests already hold the exclusive guard. G44 has a private
cache and a different production ownership key. Apple retains its shared
guards and deliberate concurrent-build scenario.

No production waiting, refusal, resource cap, test assertion, expected
value, helper, test count or guarantee changes. This is fixture scheduling
for existing rows 12, 17, 23 and 26. Their named sabotage and outside
expected values are unchanged. The change fixes collision scheduling; it
does not claim to fix performance or admit the suite budget.

## Measured critical-path windows

The CI timestamps provide these wall-clock windows. They are not individual
BuildKit phase measurements or CPU accounting. In particular, the first
window combines cold setup and work before the first test completion.

| Window, UTC | Duration | Observation |
|---|---:|---|
| 13:05:10.157 to 13:05:14.603 | 4.446 s | Wrapper/test compilation and startup before 27 tests begin. |
| 13:05:14.603 to 13:07:00.308 | 105.705 s | First test completion; cold setup and G23's exclusive prefix overlap this window. |
| 13:07:00.308 to 13:09:28.112 | 147.804 s | Further shared work, including G44's completion. |
| 13:09:28.112 to 13:10:19.903 | 51.791 s | Remaining shared work through G12's completion. |
| 13:10:19.903 to 13:12:52.164 | 152.261 s | Last shared test completion to exclusive cleanup test completion. |
| 13:12:52.164 to 13:13:11.988 | 19.824 s | Remaining exclusive runtime tests finish. |

Waiting for a runtime guard is included in libtest's test duration, so
the printed sixty-second notices do not identify when a fixture actually
acquires its guard. These windows motivate measuring normal CLI preparation,
sequential VMM lifetimes and host gaps before any performance change.
No resource limit or budget increase follows from them.

The complete raw job log is preserved locally as
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/linux-pr252-30d16d9.log`,
SHA-256 `98601cb3023078fb29192c38b957e8b78534b332d5ee1cf05b9954a80966d13f`.
The bootstrap workflow passed; it does not change the runtime result above.
