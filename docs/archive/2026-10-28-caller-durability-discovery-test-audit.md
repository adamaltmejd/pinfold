# Caller image durability and config-only discovery test audit

Date: 2026-10-28. Source baseline: `b2071840ee5b1aca12720a3600a817278f586ac2`.
The test-audit approval review covers the two changed assertion scenarios in
the existing guarantee 23 test. No test function or production test seam is
added. This source extension has not been built or run by its author.

## Post-commit directory-sync failure

1. **Guarantee.** Row 23 now includes a successful caller build after an
   injected post-commit directory-sync EIO. The existing built/exit/ref/latest
   assertions and later first-ref box launch remain its outcome observers.
   The added receipt assertions require exactly one EIO and no supervisor
   error; they establish that the fault occurred in the real CLI run.
2. **Sabotage.** The test comment names returning the directory-sync error
   from `catalog::commit` as a failed build. The first build would then report
   failure after publication, violating its existing built/exit assertions.
3. **Gap and hardest case.** Ordinary successful and failed recipes never
   reach this host filesystem-error branch. The external Linux supervisor
   selects the real notifying fsync FD by directory device/inode and requires
   the catalog inode to have changed before returning EIO. All other fsyncs
   continue in the kernel. Linux holds the existing exclusive runtime guard
   throughout the test to exclude other suite writers. The supervisor uses
   kernel notifications and pidfd readiness, with no sleep or timing guess.
   The second and third builds continue through the ordinary unsupervised CLI.
4. **Outside expectations.** EIO comes from the host Linux errno definition;
   injection count one and supervisor error zero come from the external
   fixture's contract and private receipt. Success after commit comes from
   the spec. Expected `/marker.txt` bytes remain the host-written `first`
   fixture. Ref/latest/image-id equality is the guarantee that those public
   outputs agree; the later real box reads the first ref's expected bytes.
   No warning prose, syscall argument layout, or production file format is
   asserted. This scenario makes no cancellation or physical crash proof claim.

The canonical C source is `crates/e2e/tests/e2e/fsync_eio.c`, 231 lines,
SHA-256 `42f5d3670a094221c3af9f34a4d3b775d9987b84fa5eb97dce99f01c0cec70c9`.
It was moved byte-for-byte from the untracked manual proposal, rather than
copied. The supervisor changes a real kernel syscall result; it does not
replace a runtime, guest, CLI result, image, or production consumer. It adds
no binary flag, internal import, or shared library hook.

## Config-only external base

1. **Guarantee.** Row 23's caller recipe uses a zero-layer external base's
   configuration, then copies its executable toolchain from a local image.
   Its real RUN checks `foo=bar` before the existing stamp, COPY and ONBUILD
   instructions. The existing successful-build and launched-marker assertions
   observe the complete normal path.
2. **Sabotage.** The comment names discovering only surviving LLB image
   SourceOps again. Execution's default-deny policy would refuse the missing
   config-only base identity. Dropping its configuration instead makes the
   fixture's RUN check fail. Either change fails the first built assertion.
3. **Gap and hardest case.** The previous complete local base retained a
   filesystem source operation and could not exercise native frontend
   replacement of a zero-layer image with scratch. The revised Linux recipe
   requires both external metadata and local layer sources, while preserving
   the existing later ONBUILD parent/child control. Apple keeps its original
   recipe and shared runtime guard.
4. **Outside expectations.** The immutable DockerHub manifest is
   `tonistiigi/test:nolayers@sha256:acf1bec0759497a99077fe034707bb64ef675aee61e9902e26af53aeb1c22e0c`.
   Its independently inspected 319-byte config is
   `sha256:78f8bb0557f61538b4fbb173d0ca3a8f466d77cfeee0c62a1a6f969288eeff6b`:
   Env includes `foo=bar`, and rootfs diff_ids is null. The 315-byte Docker
   schema 2 manifest has null layers. This expected value is not calculated
   from Pinfold output. Pinned upstream BuildKit commit
   `f5b59a1f2fb25a0cb7072c398cab9aed7835da4e` already uses this image in
   `client/client_fileop_test.go:26` and `client/client_test.go:327`.

The parent session independently ran the normal CLI zero-layer build and
launched its `foo=bar` result using the new bootstrap. It also ran the actual
post-rename EIO supervisor: one injection, built/exit zero, and a launched
literal marker. The durable evidence bundle names supplied to this review
are `local-zero-proof-partial.tar.gz` and `durability-continuation.tar.gz`
under `target/phase6b-builder-integration-2026-10-28/`. These earlier proofs do
not constitute a run of the final edited guarantee 23 test.

## Source and maintenance cost

At this source snapshot, the Rust test diff is 66 added / 10 removed lines.
The C fixture adds 231 canonical lines. Receipt setup/assertions are at
`box_.rs:1530-1566`; the changed recipe is at `:1513-1524`; the ordinary CLI
command constructor shared by both paths is at `:2003`. No lib.rs helper is
added. Existing row 23 box/ref assertions are reused.

The production mechanisms beside those test lines are
`catalog.rs:749-779` (31 lines, publication commit/durability outcome),
`builder-root/discover/main.go` (242 lines, supported native resolver receipt),
`catalog.rs:104-160` (57 lines, receipt admission), and `job.rs:554-667`
(114 lines, frozen local selection and external execution policy). These
are mechanism spans at the named source snapshot, not claimed diff counts.

The fixture compiles once with the already installed host C compiler and
Linux UAPI headers. It adds no build or box beyond the existing first caller
build, but supervises that build's fsyncs and serializes the full Linux row
against other runtime users. Its 231-line kernel/process cleanup burden is
material. The parent must measure the final Linux suite against the existing
five-minute budget before accepting that cost, and run the final Linux and
Mac suites. Rust formatting, the unchanged C source hash and diff whitespace
checks passed; no compile, suite, VM or sabotage run was performed here.
