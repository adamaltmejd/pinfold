# Bundled tools and publication delay

Added AnyDoc 0.2.4 to the bundled default profile, alongside Poppler.
The default's embedded skills describe local conversion and PDF page
inspection. No Node or npm installation is needed: the wrapper invokes
Bun and the matching checksum-pinned Linux native package directly.

Nightly repinning now has two tiers. Pi, claude and codex take upstream's
latest stable version without an age hold. Bun, rtk, ponytail and AnyDoc
take the newest eligible stable version after seven full days. The helper
uses publication times, not when pinfold first saw a release. Every
required package or asset must qualify. A GitHub asset replacement resets
its wait. Existing pins are retained when no eligible upgrade exists;
some pre-policy pins are younger than seven days and are not downgraded.

The helper uses Python's standard library, already required by the bump
script. It verifies publisher SHA-256 or strong npm integrity before
emitting pins. Same-version pins keep their existing URLs and checksums.
The shell stages both pin files after all metadata and downloads succeed.
The nightly guard now also covers unreleased updater implementation.

Debian packages and gh retain their current signed-repository build-time
updates. A publication delay for them would need separate package pins or
repository snapshots. User profile files are not managed by nightly CI.
The previously installed default and research profile files were preserved.

## Verification

On macOS ARM64 with Apple Container:

- Built the real source binary and its bundled default image with isolated
  XDG configuration, avoiding the installed user default override.
  The resulting image digest was
  `sha256:caba41d792232d6ae647b2a00db764462fed6cc4305dcd6f8b125eef906f65fe`.
- Guarantee 32's real box converted the host PDF and RTF fixtures with no
  egress, inspected PDF metadata, extracted page two and rendered its PNG.
- Guarantee 14's routed model fixture observed both bundled document
  skills in pi's request. Both targeted tests passed.
- 58 fixed-time scratch checks covered the exact seven-day boundary,
  timezone offsets, malformed/missing/future timestamps, stable-channel
  ceilings, younger latest releases with eligible older versions,
  preserved existing pins, GitHub draft/prerelease filtering and
  pagination, asset replacements, matching AnyDoc native packages and
  integrity failures. Six additional scratch cases exercised the shell's
  actual stable-version validator for the main tools.
- A live metadata run against a temporary Containerfile copy exited 0,
  emitted eight pin rows and changed none. The source file stayed identical.
- Cargo formatting, Clippy with warnings denied, Ruff lint/format, shell
  syntax and diff whitespace checks passed. ShellCheck reported existing
  SC1007 and SC2013 findings, also reproduced on the committed baseline.
- `PINFOLD_AUTO_RELEASES` was confirmed `true` through the repository
  variable. The existing nightly gates were preserved.

Scratch checks are maintenance-script verification, not binary E2E tests.
No permanent unit tests, mocks or binary test seams were added. The live
metadata check and real image tests provide the practical source and
runtime verification available on this host.

The full Mac suite, Linux suites, x64 image execution and release builds
were not run. No release was created and the installed CLI was not replaced.
The candidate build moved the shared default image tag; the running
research box retained its own image and was not interrupted.

## Test audit

| Guarantee | Disposition | Outside observer and expected value | Sabotage |
| --- | --- | --- | --- |
| 32 | Extended the document-reading test with AnyDoc PDF and RTF conversion | Host output files contain fixture text. Poppler's page count, selected text and PNG dimensions come from the host PDF. Input has spaces and reordered page objects; the box has no egress. | Remove Poppler or AnyDoc's native package from the bundled image. |
| 14 | Extended the existing config-level test | The host model fixture sees skill names from the profile's source front matter. The existing custom fixture skills do not cover embedded document skills. | Omit either document skill from `DEFAULT_SHARE`. |

The reviewed blocks are `crates/e2e/tests/e2e/pi.rs:25-143` and
`:756-864`, beside `profile/Containerfile:7-82` and
`crates/pinfold/src/core/profile.rs:24-41`. No duplicate assertion block
was added for profile loading or document conversion.

Commands:

```sh
cargo test -p e2e --locked --test e2e pi::the_default_profile_reads_documents_locally -- --exact
cargo test -p e2e --locked --test e2e pi::both_pi_config_levels_load_behind_a_route -- --exact
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
ruff check scripts/image-tool-pins.py
ruff format --check scripts/image-tool-pins.py
sh -n scripts/bump-pins.sh
```

## Review rework

A high-effort review of PR #79 reworked the candidate before merge:

- `scripts/bump-pins.py` replaces `bump-pins.sh` and `image-tool-pins.py`.
  One resolver per source (GitHub, npm, claude's manifest) serves both
  tiers, and the files are rewritten by exact text replacement instead of
  awk: 434 lines of shell and Python become 303 of Python.
- Harnesses follow the image tools' rule: a pin only moves up and is never
  rehashed at its version. The shell re-downloaded every harness asset
  each night, and would have silently repinned a same-version asset
  replacement or followed a yanked latest downward.
- An AnyDoc version whose optionalDependencies name other native versions
  is ineligible rather than aborting every nightly. A tool with no
  eligible newer version prints `<tool> <current>: <latest> is not yet
  eligible`, which is also how a renamed asset shows. Upgrades print
  `<tool> <old> -> <new>`.
- `read-pdf` and `convert-documents` became one `read-documents` skill.
- AnyDoc installs in the bun/rtk RUN step; its archives are named by
  `aarch64`/`x64` like theirs.

Verification on macOS ARM64:

- Fake-upstream scenarios: all current (no downloads), harness updates,
  codex latest missing an asset (waits, no paging), the seven-day edge,
  paging to an older bun, a replaced asset, a prerelease, an AnyDoc
  version mismatch, a deprecated version, a young native package, a bad
  timestamp, an integrity failure leaving both files untouched, no
  downgrade, and a prerelease latest refused.
- Live: unchanged pins produced no output and no change. With every pin
  rolled back to a fake older version, the script restored the harnesses,
  bun and AnyDoc byte for byte. rtk and ponytail stopped at 0.50.0 and
  4.10.0, since 0.51.0 and 4.12.0 are under seven days old.
- Guarantees 32 and 14 passed against a fresh build of the reworked
  Containerfile (56 s). `cargo fmt --check`, Clippy with warnings denied
  and Ruff passed. The full Mac suite and Linux suites were not rerun.
