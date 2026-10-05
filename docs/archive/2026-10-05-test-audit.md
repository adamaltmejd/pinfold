# Document tools PR test audit

Candidate review for the bundled local document readers and two-tier
nightly repinning, based on main at `0d24c73` (pinfold 0.1.6).

## Dispositions

| Guarantee | Disposition | Outside observer and expected value | Failure proof |
| --- | --- | --- | --- |
| 32 | Keep the new document-reading test | Host PDF and RTF fixtures supply text markers. The PDF page tree defines order; its pages have different dimensions. A real box with no egress writes Markdown and a PNG back to the host. | Removing Poppler or AnyDoc's native package from the bundled image makes commands fail. Rendering the wrong page produces different PNG dimensions. |
| 14 | Keep the two bundled-skill assertions in the existing test | The host model fixture sees skill names taken from the source front matter. Existing custom profile/project markers do not cover binary embedding of these skills. | Omitting either skill from `DEFAULT_SHARE` removes its name from the request. |

Two candidate findings were fixed before the host gate:

- The document test originally reused the shared default image. It now
  copies the current binary's bundled default with `profile new --builtin`
  and builds a dedicated image, so cached older tools cannot hide a removal.
- The fixture originally gave both pages identical dimensions. Page one
  is now 300 by 150 points; page two remains 200 by 100. The selected-page
  render assertion can distinguish them. The byte-length-preserving edit
  leaves the fixture's cross-reference offsets valid.

No existing assertion was weakened or deleted. No unit tests, mocks,
runtime stubs or binary test seams were added. The scratch updater and
package-manager evaluations are recorded separately as maintenance-tool
verification, not binary E2E proof.

Test lines: `crates/e2e/tests/e2e/pi.rs:25-152` and the bundled-skill
assertions in `both_pi_config_levels_load_behind_a_route`.
Binary/profile lines: `crates/pinfold/src/core/profile.rs:24-41` and
`profile/Containerfile:7-82`. The final read-only candidate audit found no
remaining issues in these blocks or the updater/workflow diff.

## Host gate

On macOS ARM64, with no competing runtime suite and the user's research
box preserved:

- `cargo test -p e2e --locked`: 32 passed, zero failed or ignored;
  E2E wall time 222.24 seconds. Includes the fresh-image document test,
  routed skill-loading test and live codex login route.
- `cargo fmt --check` and
  `cargo clippy --all-targets --locked -- -D warnings`: passed.
- Ruff lint and formatting, shell syntax and Git whitespace checks: passed.
- Live updater metadata verification retained all eight current image pins
  and did not change the source Containerfile.
- ShellCheck's SC1007 and SC2013 findings match committed main. Actionlint's
  three unknown runner-label findings also match main. Neither linter added
  a candidate finding; those baseline findings were not changed here.

Linux x64 and ARM64 verification belongs to PR CI. Release builds and the
whole-tree release cleanup pass were not run; this is a PR, not a release.
