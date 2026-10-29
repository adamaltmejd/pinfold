# Critical cleanup test cut

Date: 2026-10-29. Adam explicitly approved reducing G15 to two case
builds and narrowing its executed-coverage description. This is a
coverage reduction, not a claim that the deleted scenarios were duplicates.
Production behavior and security controls are unchanged.

| G15 block | Disposition and outside expected value |
|---|---|
| Live/idle project cleanup | Retain one project image. Host fixture state bytes and the guest's actual disk write survive dry-run; both disks survive plain clean. Real unused cleanup removes idle state/disk and preserves live state/disk bytes. |
| Caller image lifetime | Retain one caller build. Runtime inventory and a running box observe its image surviving removal. After teardown, the same removal reclaims the refs and Linux's observed backing file. |
| Apple external resources | Retain the foreign native container using the shared default image and a genuinely unlabeled native image. Both have positive existence controls and survive cleanup. |
| Four profile builds/newest-two retention | Delete the approved dedicated matrix and its assertions. Production retention remains unchanged. |
| Dead project image/label matrix | Delete the second project build and dead-box setup; row 8 retains owner-loss/pruning coverage. Idle state/disk cleanup stays here. |
| Three retired images, prefix sibling and shared-ID twin | Replace five caller builds with the one live/stopped image case. Remove the sibling/tag-sharing and prefix assertions from row 15's coverage. |
| `labeled_images` and unused imports | Delete the helper orphaned by the removed retention matrix. |

Named sabotage covers destructive dry-run, ignored live-project protection,
omitted idle removal, premature image/root deletion and retained roots
after removal. The hard cases are actual live writes across cleanup and
the same image removal before and after its box stops. Expected bytes
come from fixtures, existence from host/runtime inventory, and root
lifetime from the observed host backing file.

Linux has two case builds: one project CLI build and one caller CLI build.
Apple retains its native project-image fixture and caller CLI build,
plus one separate native unlabeled-image fixture. Shared default setup is
excluded from these counts. The extra Apple fixture is disclosed rather
than counted as a removed image build.

`runtime.rs` shrinks from 1,286 to 981 Rust lines. Pinfold's production
Rust source remains 18,096 lines and is unchanged. Other runtime tests
are byte-identical. G44 and its fixtures are unchanged. ARCHITECTURE.md's
row 15 describes only the retained execution; its production maintenance
contract is unchanged.

Source review and formatting completed. No build, suite, VM or public
push was performed for this cut. Platform execution and timing remain
pending; fewer source calls are not measured speedup evidence.
