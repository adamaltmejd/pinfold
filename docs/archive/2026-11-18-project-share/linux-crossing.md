# Linux disk crossing recheck, 2026-11-18

The dev VM ran the real x86_64-unknown-linux-musl debug binary at
`3e7be92395f255ea76aca6a5e0ce64afbe9cb197` with private project and XDG
directories. These observations come from the completed session transcript.

| Fixture and commands | Observations |
|---|---|
| Running disk owner. Commit in guest and on host, then run `pinfold fetch` and `pinfold push`. | Both exited 0 and named the same box. Host Git read guest commit `09c1f2b` at `pinfold/main`. Guest Git read host commit `a3e1c14` at `host/main`. |
| Stop owner, commit on host, repeat fetch and push. | Both exited 0 in distinct transient boxes. Fetch reported `09c1f2b`. Guest Git subsequently read `e2a47f1` at `host/main`. No Firecracker process remained after either crossing. |
| Run both verbs from an independent child repo under a running parent disk. | Without child config both exited 1 with `no-config`. After `config edit` both exited 1 for the missing child disk. After initializing the child and committing separately, both exited 0 using child boxes. Host Git read `a707e9b` at child `pinfold/main` and guest Git read `862ce29` at child `host/main`. Parent host and guest refs kept their values from the parent control. |
| Run both verbs from a nested linked worktree with the parent owner running, then after `pinfold stop`. | All four calls exited 1 with `worktree`. Host refs were unchanged. |
| Set the project mount to share and run `pinfold start` without an owner. | Exited 125 with `spec: share mounts are unsupported on Linux`. No Firecracker process started. |

Cleanup removed the fixture, private state, runtime directory and build log.
The checkout was clean at the tested commit. This run did not exercise a live
Linux disk owner after its configuration was edited to share.
