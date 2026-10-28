# ONBUILD platform scope and assertion review

Date: 2026-10-28. Base: `a75e0418b9929b4e407f5d19863b2c72c931b318`.

The parent session's actual Mac caller-image run passed the parent marker's
absence check. Its child build also succeeded, but the child had no
`/onbuild-marker`. The native Apple builder tested there did not execute
this inherited trigger. No Apple production code changed in this work.
This observation narrows the new scenario; it does not establish behavior
for other native-builder versions or authorize an Apple parser or workaround.

The existing guarantee 23 test now appends ONBUILD to its caller recipe
only on Linux. The parent absence and child execution assertions are also
Linux-only. All earlier Mac COPY, identity, retention, cache and failure-tail
cases remain. The earlier ONBUILD audit remains a record of the wider
candidate before this platform result.

The four assertion-review answers are:

1. Extend guarantee 23 and its existing caller-image test. The guarantee
   row now explicitly qualifies ONBUILD deferral and execution as Linux-only.
2. Using the published configuration directly during Linux FROM-only
   conversion makes the parent marker appear prematurely. Removing the
   trigger from the published configuration loses the child marker. The
   existing sabotage comment names both changes.
3. The previous cases exercise COPY and cached RUN layers, not inherited
   instructions. Launching the first immutable parent ref after latest
   moves, then building a child from that exact ref, checks both deferral
   and preservation of the trigger. The child is the positive control.
4. Expected marker bytes come from the fixed caller recipe, and timing
   comes from guarantee 23. Ordinary CLI build/launch/exec operations
   observe the guest filesystem. No internal import or new test seam is used.

The platform restriction changes only fixture selection and the two new
assertion blocks. It adds no test or helper and leaves the Linux sabotage
and external expected values intact. Formatting passed. No runtime run
accompanies this note; the parent session must rerun the affected Mac
case and complete the final-head gates before pushing.
