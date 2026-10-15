# Independent implementation review: effective hook aliases and mount admission

Reviewed worktree `/Users/adam/.codex/worktrees/pinfold-lean-implementation/pinfold` at exact HEAD `a7495da045010d9dcabd8f944a0342dc9782d9b4`, base `a1be921`. Tracked source was clean throughout; final status showed a separate untracked investigation README. Targets: `cfd2eb417a756d20bc8bf094a06f74da764daa7d` (B1) and `c9677e7d0129c66bc7b9a58bd6316cfbba516823` (O1). Read AGENTS, binding ARCHITECTURE, test-audit, complete pi/git.rs, complete existing G11 (pi.rs:560-970), relevant startup/mount scan, mount normalization/runtime construction and G22 alias cases. No edits to repository files; no builds/tests/runtime operations.

**No blocking source findings in either target or their current integration.**

## B1: effective Git hook aliases

`pi/git.rs:110-150` keeps Git's own hooks-path resolution, then checks absolute spellings outside the canonical project. An existing referent inside the project refuses as protected-path-invalid rather than silently substituting a resolved mount. For missing leaves, the walk reaches the nearest existing ancestor; an in-project ancestor refuses before creating any protected directory. A dangling final alias follows its read_link target before classification. A missing leaf under an aliased parent is shortened until that parent can be canonicalized. Existing and provably outside missing paths remain outside the mount plan.

`pi/git.rs:123-142` fails closed on canonicalization/stat/read_link errors. Missing paths with unresolved `..` refuse, including traversal introduced by a previously followed dangling target. It does not infer an outside location after discarding unresolved parent traversal. This is intentionally conservative: an ambiguous missing outside spelling may refuse. The new G11 parent-traversal scenario documents that choice rather than treating it as an allowed external path.

Ordinary missing components strictly shorten via parent(). An unparented root returns an error. Static symlink cycles produce canonicalization errors which propagate. There is no ignored resolution failure, retry, target creation or unbounded reader. No atomic protection is claimed against concurrent host path mutation.

Lexically inside and relative paths retain `protected_path`, path_kind's no-symlink-in-components validation, root/escape refusal, `.git` already-covered omission, and the preparation cleanup guard. The project root supplied by pi launch remains canonical. The B1 paths/test files are unchanged between their target commit and reviewed HEAD.

## O1: read-only file identity index

`core/box.rs:624-638` clones the plan's mounts, normalizes guest destinations, applies the existing runtime init masking rule, then sorts only this scan-local copy so read-only identities precede writable comparison. Actual `plan.mounts` and the runtime's mount construction (`runtime/mod.rs:556-566`) retain their previous order and contents.

`core/box.rs:655-663` determines descendant masking from the full normalized destination set. Sorting therefore cannot change which occurrences are masked. A read-only parent with a writable descendant excludes that descendant during read-only traversal; a writable parent with a protected descendant excludes the protected occurrence during writable traversal. An additional export remains separately traversed. Exact init destinations are removed only from the scan copy as before; duplicate caller destinations were validated before preparation.

`core/box.rs:664-680` uses symlink_metadata below canonicalized mount roots, so it still follows only mount-root aliases and compares regular files by device/inode. Every unmasked writable occurrence is checked against every indexed effective read-only identity. Read-only/read-only and writable/writable duplicates need no conflict record. Single-link files exported twice remain checked; hardlinks outside writable exports remain allowed. No nlink shortcut, traversal omission or alias admission weakening is introduced.

Cancellation checks remain per mount, directory and entry (`641-652`). Canonicalization/read_dir/entry/stat errors still propagate. The async preparation handle and cancellation cleanup (`box.rs:299-310,529-542`) remain owned and awaited; the scan finishes before proxy/runtime creation. The existing source qualification remains binding: this observes layout at admission, not ongoing or atomic enforcement. Later HEAD changes in box.rs concern Apple readiness metadata reuse and passing the claimed state directory into runtime startup, not the identity-index mechanism.

## Test-audit gate

Guarantee 11 and the new clause at ARCHITECTURE.md:951 cover B1. New pi.rs:635-675 assertions observe real CLI exit 1, the spec's protected-path-invalid token and host fixture hook bytes. Named sabotage restores hooks_path's former lexical absolute-outside early return. Existing direct hooks-path and protected symlink blocks would not enter that branch; the missing descendant and dangling target are harder cases than an existing direct directory. The unresolved-parent scenario covers the conservative absent-path branch.

The executable host hook and actual host Git commit (`pi.rs:677-698`) independently prove that the alias reaches the hook; its marker and bytes come from the fixture. The genuinely external control (`700-707`) runs the same project's real Pi --version launch, which passes `Git::prepare` in pi/launch.rs:36 before starting the box. Restored direct in-project hooks then retain a read-only write refusal and writable-project control in the same live box (`714-797`). No new test function, internal import, test-only binary flag, copied oracle or twin assertion appears.

The rest of the complete G11 keeps distinct controls for Git/editor hardlink admission and cleanup, Git config/hook/mount-point writes, configured protection, symlink refusal/repair, late non-UTF-8 preparation cleanup, launch inside .git, and a space-named root. No deletion is indicated in the reviewed target assertion blocks. **Lean already.**

O1 changes no assertion. Existing G22 covers reversed mount order, an additional writable export, a single-link duplicate export, normalized guest spelling, and an outside hardlink positive control. G11 separately exercises Pi's prepared Git/editor protections, so its alias blocks are not twins of the caller-owned mount fixture.

## Limits

Source approval only. No native path behavior, cancellation timing, memory/performance result, or runtime gate is established by this review. Parent owns real Mac/Linux G11/G22 and all required validation. Static correctness does not remove the specified host-mutation window after admission, or extend protection to planted repositories/worktrees outside the current contract.
