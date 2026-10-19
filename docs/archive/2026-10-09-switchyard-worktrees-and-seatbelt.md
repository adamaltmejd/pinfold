# Switchyard worktrees and Seatbelt

This follow-up records the clarified use case and a native Seatbelt probe.
It supersedes the scope-first recommendation in
[the earlier research](2026-10-09-worktree-security-and-startup.md) for
Switchyard's general agent workflow. The binding architecture is unchanged.

## Required workflow

Switchyard needs to launch contained agents against complete existing Git
worktrees, including root manifests and arbitrary source directories, on
the user's existing case-insensitive Mac filesystem. Startup should not
copy the checkout or its history. Interactive `pinfold pi` remains separate,
including its configured GitHub access. Scoped directory editing does not
meet the general Switchyard requirement.

## Seatbelt is a candidate requiring integration evidence

A native probe used a temporary regular file named `.git`, its `.GIT`
case alias, a preexisting hardlink named `alias`, and an ordinary writable
control file. The profile was `(version 1)`, `(allow default)`, and a
`file-write*` denial for the literal absolute `.git` path. Each operation
ran in a fresh process through macOS `sandbox-exec`.

| Operation | Observed result |
|---|---|
| Write ordinary control file | Succeeded; protected bytes unchanged |
| Write `.git` | Operation not permitted; bytes unchanged |
| Write `.GIT` | Operation not permitted; bytes unchanged |
| Rename `.git` | Operation not permitted; original remained |
| Write preexisting hardlink `alias` | Succeeded; protected bytes changed |

This demonstrates that a native Seatbelt rule can block the case-alias
write that defeated the guest pathname mask. It also demonstrates that
this literal rule is not complete inode protection. No actual Git command
or repository metadata was used in this probe. It is not evidence that
Seatbelt confines a VM's filesystem service.

[Apple Container 1.5.0's architecture](https://github.com/apple/container/blob/1.5.0/docs/technical-overview.md)
separates the CLI from its launch-agent API server and per-container runtime
helpers. Sandboxing only Pinfold or the CLI does not establish restrictions
on filesystem operations performed by those independently launched services.
Integration must reach the host process actually servicing guest file I/O
and verify any delegation inside the Virtualization framework. The public
shared-directory interface exposes directory-wide read-only access, not a
per-file deny policy.

The next useful experiment is therefore a contained runtime integration
spike, not another CLI wrapper: identify the actual writer, determine how
to apply a per-box Seatbelt policy before sharing writable files, and test
the policy through the real guest. It must protect pointer replacement,
case aliases, metadata aliases and overlapping exports while ordinary root
file edits succeed. No working integration is established yet. Running
the agent directly under native Seatbelt is a separate runtime design and
would replace the current Mac VM/Linux environment boundary.

## No Git and caller-owned commits

The viable requirement is that the agent cannot modify, replace or redirect
host Git control files. Absence of a Git executable or omission of the
common Git directory does not establish that requirement: ordinary file
writes can poison the pointer, and a later host process follows it.
Removing or hiding `.git` must also prevent creation or replacement at the
same host name and its aliases.

The historical decision rejected a broad host Git command shim, but
explicitly retained a fixed snapshot-commit operation as a possible later
experiment. See [the recorded decision](2026-09-23-grilling.md#git-read-only-git-replaces-the-handoff).
It is inaccurate to say that every host-side commit interface was rejected.

A narrow Switchyard commit operation could bind its repository, worktree
and expected parent to trusted job state, accept bounded data rather than
arbitrary Git arguments, and control configuration and environment so Git
does not execute hooks, signing programs, fsmonitor commands or content
filters. Exact semantics and tests remain design work. Short tasks may
only need Switchyard to commit after the box has stopped, with no live MCP
endpoint. MCP is a transport choice, not the security mechanism.

Even an independently hardened commit operation would not protect an editor,
shell prompt or other host Git process from a poisoned `.git` pointer.
The filesystem protection problem remains. Git's own
[security guidance](https://git-scm.com/docs/git#_security) explains why
agent-controlled Git metadata is unsafe for host Git operations.

## Revised direction

Prioritize host-side protection of full worktree exports, with Seatbelt
as a concrete candidate to validate. Keep Switchyard responsible for its
Git workflow and Pinfold responsible for enforceable file/process/network
boundaries. Do not add a scoped mode to interactive pi, require drive
reformatting, or treat no-Git as permission to expose a writable gitfile.

A separate case-sensitive disk image would not require reformatting the
Mac, but changes storage placement and is not the default requirement here.
If runtime-level protection cannot be established, isolated working files
with checked application of changes remain the larger fallback. No source
code, runtime configuration or existing worktree was changed.
