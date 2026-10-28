# Phase 3 assertion audit

Date: 2026-10-28.
Scope: the image/profile feature's changed assertion blocks. This is not
the release-wide sweep. No test was added. No sabotage was executed.

Ten blocks were extended or added within guarantees 12, 17, 18 and 39.
All ten are kept; none is deleted. The existing project-recipe scenario
now uses a recipe-less profile. The remaining existing assertions stay.

| Row | Test lines | Binary lines | Disposition and evidence |
|---|---|---|---|
| 12 | `pi.rs:526-544` | `config.rs:64-100`, existing `trust.rs` exact-byte check | Keep. Adding an image that selects the prior effective reference still requires approval. Existing domain/recipe changes did not cover the new selector. The fixture provides the bytes; the CLI supplies the refusal token and a permitted launch after approval. |
| 12 | `pi.rs:545-549` | `pi/launch.rs:309-320` | Keep. An explicit image overrides a deliberately unrelated profile recipe without an `image-outdated` token. Comparing that image to the profile recipe is the named sabotage. |
| 12 | `pi.rs:550-558` | `cli.rs:1053-1067` | Keep. After approval, an explicit selection refuses build as `image-no-recipe`. Before approval the same command refuses as `untrusted`. The existing project recipe is the successful build control. |
| 12 | `pi.rs:566-594` | `cli.rs:1074-1112`, `core/profile.rs:195-199` | Keep the existing block with a harder fixture. A changed project recipe still stops launch/build, then builds and runs after approval, while its profile supplies no recipe. Expected recipe bytes come from the host fixture. |
| 17 | `box_.rs:301-335` | `core/box.rs:779-790` | Keep. A stale managed image tag must not bypass missing-source admission on a recipe-less profile. The same box/profile with an explicit local image starts and answers exec. Existing missing-image refusal could not detect fallback to a stale valid tag. |
| 18 | `pi.rs:1123-1189` | `config.rs:53-100`, `cli.rs:959-979` | Keep. Offline source selection covers profile recipe, layered profile image, project image override and a project recipe on a recipe-less profile. Expected sources and references are the declared fixture inputs. Host tree snapshots prove read-only config for these new paths. |
| 18 | `pi.rs:1191-1241` | `config.rs:78-97` | Keep. A lower-layer image plus project recipe catches validation before layering; missing and empty image selections assert their distinct tokens. Correcting each fixture permits config. Host trees stay unchanged for config; working-command refusals create no box/project/disk/run state. Ordinary maintenance remains allowed. |
| 39 | `pi.rs:733-753` | `core/profile.rs:185-229`, existing backend preparation | Keep. The live config-only profile supplies a home seed and shared file, refuses a guest share write, and preserves host bytes. The fixture supplies all expected bytes. Omitting seeds/share or allowing share writes is the named sabotage. |
| 39 | `pi.rs:914-917` | existing `core/box.rs` launch record | Keep. After the project's selected image changes during a run, inspect retains the launched ref and runtime-observed image id. Runtime inventory supplies the expected id. Current-config inspection is the named sabotage. |
| 39 | `pi.rs:964-995` | existing missing-only seeds and profile application | Keep. A second launch sees host-updated shared files and preserves a guest-edited seed. A profile seed edit after allow proves host-profile authority without reapproval. Overwriting seeds, reusing stale share bytes or recreating disks is the named sabotage. |

The test diff adds 250 lines and removes five. The feature binary diff adds
116 lines and removes 27. Added test code includes fixtures and comments;
no helper or binary seam was added for a test. Runtime validation belongs
to the main implementation session and its final-head PR evidence.
