# Manual fresh-state validation

Date: 2026-10-28. Phase 5 of the approved refactoring plan.
Code baseline: `d7656d7` (phase 3 merged). Documentation-only change.
`docs/ARCHITECTURE.md` remains the spec. Caller changes: None.

## Scope and method

The README adds the existing manual disk workflow. No binary, assertion,
guarantee, dependency or profile changed. This does not depend on phase 4.

A current-main binary was built with
`RUSTC_WRAPPER= cargo build -p pinfold --locked`.
The first build hit a sandbox-bound sccache write failure;
disabling that wrapper succeeded. Two initial driver attempts corrected
inventory arguments and the guest exec working directory. They exposed no
binary defect and removed their disposable disks through the CLI.

Manual validation used real Apple Container 1.6.1 on macOS 27.0.1 (26A434),
real pinned Pi 1.4.0 and an existing local default-profile image. A disposable
Git project, separate state/config roots and a copied fixture profile kept
operator project state out of the exercise. A local host HTTP fixture
returned a fixed OpenAI-compatible stream through a Pi login route, using
only a public fixture placeholder. No real model service or credential was
used. This follows the existing guarantee 26 fixture pattern.

The operator reserved the Mac runtime slot. The driver also held the
existing `/tmp/pinfold-mac-suite.lock`. Before and after, runtime inventory
showed only the pre-existing BuildKit container. The slot is released.

Six `pinfold pi --mode rpc` invocations waited for `get_state` replies.
Guest observations used `pinfold box exec` with an explicit project cwd.
Three `pinfold pi -p --provider openrouter --model openai/gpt-4o-mini`
invocations reached the fixture and returned `lifecycle-ok`, ending normally.
The first two prompt runs produced session files observed in later boxes.
Three detached `pinfold fetch` calls used the existing transient-box path.

## Observations

The host source commit was `5f6e6533494a3a37bf21208f078c71158b3cdb3f`.
A guest commit `d543b5b8390730d84169bd6676cde78c89e39e10` added
`guest-committed.txt` containing `guest-commit`. A separate untracked guest
file contained `guest-dirty`. The home had a changed seed, changed settings
and a home-only marker. Host untracked input never appeared in the guest.

| Case | External observation after launch |
|---|---|
| Persistent RPC, prompt, RPC | Guest HEAD and dirty file survived. Changed seed, home marker and settings hash survived. The prompt created a readable session. |
| Home disk removed | Guest HEAD and dirty file survived. Home marker and prior session disappeared. Seed and settings returned to profile bytes. |
| Project disk removed | Guest HEAD returned to the host source commit with a clean worktree. Guest committed and dirty files disappeared. The second home marker, changed seed, settings hash and actual session file hash remained identical. |
| Both disks removed | Guest HEAD returned to the host source commit with a clean worktree. Guest files, home marker and prior sessions disappeared. Seed and settings returned to profile bytes. A subsequent prompt succeeded. |

While the first Pi box was running, `disk rm` for each disk exited 1 with
`reason: disk-in-use`. Once Pi finished, removal reported `existed: true`.
`disk list` showed the selected disk absent and the other disk retained.
After removing both, it listed no disks. The next launch recreated them.

Every deletion used names read from `pinfold config`:
`pi-project-b0840a25e936-home` and `pi-project-b0840a25e936-project`.
All RPC boxes reported image id
`6d7509f79fb54099f271330942a911d71a82eba394fad33d428c532f25c1ef76`
at `localhost/pinfold/profile-default:latest`. Native image inspection
before and after was identical. No image build ran during the matrix.

Before and after every matrix removal, config still reported trusted project bytes
and the same disk names and built image. SHA-256 manifests of the fixture
profile and project trust record stayed identical. The profile settings
hash was `4d693c1fd3304c67d7d8bdbccc62f5b15d5c2dd19d81ae81b54dbc10b886f1c6`;
the initially edited home settings hash was
`e57a46939bd25bf4268b010f1142cef5bb7c6b41915a3b17e5034635928848cc`.
Project-only removal preserved session hash
`7bd954c1376419b5427d310bbf41c4b9f7fa1d50069646531032bfa974ef2593`.

Detached fetch initially put the guest commit at `refs/remotes/pinfold/main`
and left host HEAD and worktree files unchanged. Later fetches from the
recreated project moved that remote ref back to the host source commit.
The old commit remained readable by its recorded object id; a final host-only
follow-up created `retained-result` at that id and read `guest-commit` from
it. The README therefore says to inspect and merge or create a host branch
for wanted fetched commits. A remote ref alone is not permanent retention.

## Evidence and limits

Local evidence is preserved under `.local-evidence/phase5/` in the retained
phase-five-fresh-workflow worktree. It is ignored by Git. These links are
local; the evidence is not part of the PR.

- [Driver](../../.local-evidence/phase5/pinfold-phase5-lifecycle.py), SHA-256
  `c3a4f415b013c9ba46443d910a8b85fd388993d2230df6496fac598316919803`.
- [Command transcript and observations](../../.local-evidence/phase5/fixture/evidence.json),
  SHA-256 `a17962f92e1e7f75654faeb85db95bff3ca0cb5244c218157ca4edc8ad6f9092`.
- [Driver log](../../.local-evidence/phase5/pinfold-phase5-lifecycle.log) and
  [build log](../../.local-evidence/phase5/pinfold-phase5-build.log).
- [Manifest](../../.local-evidence/phase5/manifest.json) records each file's
  original path, size and SHA-256. Copies were verified byte for byte.
  `fixture/` also retains stderr logs and the disposable host project,
  including its `retained-result` branch. Keep these until the operator
  deliberately archives this worktree.

The final driver exited 0. Cleanup used `pinfold disk rm` for both fixture
disks. No state-file unlink or direct volume deletion was used. No fixture
box or disk remains; disposable host files and evidence remain for review.

This is manual Apple evidence, not an automated test or a Linux result.
No suite, clippy or format rerun was needed for prose-only changes.
`git diff --check` passed. Existing guarantees 11, 12, 13 and 39 cover the
component behavior; no test assertion changed, so no test-audit was invoked.

Manual deletion is not transactional. Serialize access to the project.
Deleted disks lose uncommitted and home-only files. Reset does not rebuild
images, reset trust or profiles, ensure deterministic output, or strengthen
source synchronization. Existing tolerant `sync-failed` behavior remains;
this exercise did not introduce or test a new synchronization guarantee.
