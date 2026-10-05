# Local document reading

Added Poppler PDF reading to the bundled default profile. Added a local
`research` profile to trial AnyDoc 0.2.4 with the installed pinfold 0.1.6.

## Setup

The bundled image installs `poppler-utils`. Its embedded `read-pdf` skill
describes text extraction and selected-page rendering. This needs no
extension or change to pi's read tool.

Installed user profiles at `~/.config/pinfold/profiles/default` and
`~/.config/pinfold/profiles/research`. The local default was copied from
the installed binary, preserving its existing image pins and settings,
then given Poppler and the PDF skill. It overrides bundled defaults;
future bundled profile changes need explicit merging.

Research builds on `pinfold/profile-default:latest`. Its AnyDoc wrapper
invokes Bun explicitly. The image installs the CLI package and the GNU
native package for its architecture outside `$HOME`. Both skills use
local conversion. No Firecrawl host was added to the allowlist.

The three npm archives were downloaded, hashed, and installed with
`ADD --checksum`. Version 0.2.4 SHA-256 values:

| Package | SHA-256 |
| --- | --- |
| `@firecrawl/anydoc` | `625bf6cdc24cc91eee8fbfe084c1fa56c71b17f034341d4a23bc8df0fdee31bd` |
| `@firecrawl/anydoc-linux-arm64-gnu` | `ed71661425a9959009d39ba495dbc565cc27a27f4eb0c93336b89f217e5c56b2` |
| `@firecrawl/anydoc-linux-x64-gnu` | `ca822ea3ad29a9b9ca6b7f6d2a6b3d5311153af25a6c35a7f5bdf0791fe7f2c9` |

Built both profiles with the installed CLI. Research must be rebuilt after
its default base image changes. Start it with `PINFOLD_PROFILE=research pi`.
The existing user pi box retained its original image throughout setup.

## Verification

On macOS with ARM64 Apple Container boxes:

- A research box with no egress converted a two-page text PDF to Markdown
  containing both fixture page markers, and converted an RTF fixture.
- An image-only PDF returned exit 3, naming the page that needs OCR.
- Poppler inspected the PDF, extracted its text, and rendered page two.
  The rendered PNG was inspected on the host.
- The installed CLI launched pi with the research profile against a
  local model fixture. Its request contained both `read-pdf` and
  `convert-documents` skill names.
- Both profiles resolved to built images through `pinfold config`.
- `cargo fmt --check` and
  `cargo clippy --all-targets --locked -- -D warnings` passed.
- Both targeted e2e tests below passed. The initial attempt could not
  compile because the pinned toolchain lacked `aarch64-unknown-linux-musl`;
  installing that target resolved the prerequisite failure.

Full Mac and Linux suites and the x64 research image were not run.

## Test audit

| Guarantee | Disposition | Observer and expected value | Sabotage |
| --- | --- | --- | --- |
| 32 | Added `the_default_profile_reads_pdfs_locally` | Real box without egress; host PDF gives two pages, selected text, and page dimensions. Input has spaces and reordered page objects. | Remove `poppler-utils` from the image. |
| 14 | Extended `both_pi_config_levels_load_behind_a_route` | Host model fixture observes the bundled `read-pdf` skill name in pi's request. Existing fixture skills do not cover binary embedding of this skill. | Omit the PDF skill from `DEFAULT_SHARE`. |

Commands:

```sh
cargo test -p e2e --locked --test e2e pi::the_default_profile_reads_pdfs_locally -- --exact
cargo test -p e2e --locked --test e2e pi::both_pi_config_levels_load_behind_a_route -- --exact
```
