# Y-12 observed mount alias admission

The candidate adds a Git-independent admission scan after preparation and
seeding, before proxy or runtime startup. It compares regular-file device
and inode across effective directory exports. Guest descendants mask their
parent occurrence. Mount-root symlinks resolve; lower symlinks are not
followed. Link count does not decide admission. Traversal errors fail startup.

Detected conflicts use the existing refused event and exit 1, with reason
`mount-alias` and guest paths. Pi uses its existing error conversion. The
existing claim cleanup and Git directory guard handle refusal. Prepared
artifacts and home seeds remain, as they do after other post-preparation
startup failures. Read-only runtime mounts remain in force.

This removes an observed static layout from admitted inputs. It does not
establish alias-complete isolation, atomic startup, protection against host
mutation or concurrent boxes, symlink-referent coverage, or protection
against later filesystem changes. Podman's separate resolver file and
socket exports are outside the directory scan. Worktrees remain refused.
Y-9 and Y-10 remain open for broader protection. The threat model and its
controls are unchanged.

## Test review

Only the existing tests for guarantees 22 and 11 change. No runtime bypass
probe, new test function, binary test seam or guest alias write was added.

- Row 22: host-created config aliases in the project and an extra writable
  export refuse, with reversed mount order in the second layout. Omitting
  the check or extra exports defeats these blocks. Existing runtime write
  checks cannot observe passive admission. The host supplies config bytes
  and hardlink identity. Existing refusal and cleanup helpers observe the
  event, exit, state and runtime list.
- Row 22: a duplicate directory export refuses even with a single-link
  file. Treating link count as permission defeats this block. The host
  establishes the link count and expected protected bytes.
- Row 22: after removing conflicts, the same fixture starts with a Git
  commit object shared outside all writable exports. Blanket link-count
  rejection defeats this control. Existing history, status and project
  write checks still run. The host supplies the object and expected bytes.
- Row 11: Pi's real plan refuses Git config and `.idea/workspace.xml`
  aliases. Omitting the check defeats admission. Releasing the Git guard
  early defeats absent-directory cleanup. Host bytes supply expectations;
  refusal output, the runtime list and host state supply observations.
  Removing aliases permits the existing RPC and project-write control.

The diff adds 114 test lines and removes 3, beside 87 binary lines added
and 3 removed. No unrelated assertion block changes.

## Worker validation and measurement

Linux aarch64, rustc 1.98.1. With `CARGO_HOME=/usr/local/cargo`, these passed:

- `cargo build --locked --offline`
- `cargo fmt --check`
- `cargo clippy --all-targets --locked --offline -- -D warnings`

Apple container and podman are unavailable in the worker. The Mac suite and
both Linux suites remain the host landing gate; compilation is not a claim
that their runtime scenarios passed.

Scanner cost was measured from the candidate's unmodified
`check_mount_aliases` function, extracted into a temporary timing driver.
The driver only supplies mount inputs and error types; it does not run or
replace a runtime. There is no production instrumentation. Source, driver,
fixture and raw results are snapshotted under `/yard/proof/y12-cost`.
The extracted function's SHA-256 is
`5f9502db23123e562acf663811b7c6284025d257cd9e5ff41fcd8d0d21c37d85`.

The caller fixture has a real host Git commit, the single-link protected
file, an external hardlink to its commit object, and an empty extra export.
Its host tree has 45 entries, 28 regular files and 18 directories including
the root. The prepared Pi project shape has 34 entries, 19 regular files
and 16 directories, including empty protected directories and hooksPath.
These counts come from an independent host enumeration, not scan output.
They describe accepted layouts; each negative hardlink adds one entry.

The timed plans include the actual worker binary's init export,
`/workspace/target/debug`, with 1,857 entries, 1,666 regular files and 192
directories. This unusually large development directory is traversed in
full. The checkout host tree has 2,226 entries, 1,956 regular files and 268
directories, including build outputs. In the checkout plan the init
export masks its writable parent occurrence. No files are pruned or cached
by the implementation.

Each run timed one first traversal and ten further traversals. Counts were
collected first, so even the first traversal is not a cold-cache measurement.
Milliseconds on this worker:

| Scanner build | Layout | First | Warm min | Warm median | Warm max |
|---|---|---:|---:|---:|---:|
| Optimized (`rustc -O`) | Caller fixture + init | 31.429 | 29.847 | 30.177 | 39.097 |
| Optimized (`rustc -O`) | Checkout + init | 36.408 | 35.985 | 37.127 | 40.377 |
| Unoptimized | Caller fixture + init | 30.912 | 28.611 | 31.981 | 45.783 |
| Unoptimized | Checkout + init | 45.002 | 36.532 | 37.226 | 41.714 |

These are scanner timings, not end-to-end startup timings. The Pi harness
cache is unavailable here; its traversal cost is not measured. The host
must account for the prepared Pi harness and platform filesystem at final
acceptance. No optimization follows from these measurements.
