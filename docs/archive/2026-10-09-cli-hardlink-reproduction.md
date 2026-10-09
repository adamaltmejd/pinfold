# Protected Git metadata hardlink reproduction

On 2026-10-09, installed Pinfold 0.2.2 reproduced a write through a
pre-existing hardlink to protected Git configuration on macOS 27.0.1,
Apple Container 1.5.0, arm64. Source
baseline: `dc1dfc1`; subsequent local commits changed only Yard configuration.

This exercises Pinfold's actual CLI, unlike the earlier direct runtime
and Virtualization.framework probes. It contradicts guarantees 11 and 22
for this fixture. It does not establish a Linux result or a safe fix.

## Fixture and operations

Two independent disposable repositories were created under
`/private/tmp/pinfold-cli-alias-probe/fixture-e4dnqwgg`. Each was initialized
with `git init`, with global/system Git configuration disabled. Before
launch, the host made `config-alias` a hardlink to `.git/config`. Both
names had the same inode and link count 2. No hooks, executable payloads,
model calls or host-execution markers were used.

The caller-owned case ran `pinfold box up` with the cached
`pinfold/profile-default:latest` image, 2 CPUs and 1 GiB memory. Its mount
list put `REPO/.git` read-only first and `REPO` writable second, both at
their own absolute guest paths, matching guarantee 22's existing scenario.
The probe waited for the CLI's `ready` event before executing commands.

The interactive case ran `pinfold pi --mode rpc` from the second repository,
with private state/config directories, empty `PINFOLD_ALLOW`, 2 CPUs and
1 GiB memory. It waited for the response to:

```json
{"type":"get_state","id":"probe"}
```

It selected the box by its owner PID from
`pinfold box list --label dev.pinfold.owner`. This leaves Pi's normal
Git protection assembly in charge; the probe supplied no custom mounts.

For each case, `pinfold box exec BOX -- sh -c` ran these operations,
with REPO replaced by the fixture's absolute path:

```sh
printf '%s\n' '# direct-probe' >> REPO/.git/config
printf '%s\n' 'writable-control' >> REPO/source.txt
printf '%s\n' '# harmless-alias-probe' >> REPO/config-alias
```

The host compared `.git/config` bytes and inode identity before and after.
The writes only appended comments. The source-file write was the positive
control for writable project access in the same box.

## Observed results

| Observation | Caller-owned box | Interactive Pi |
|---|---|---|
| Direct config write | Exit 2, Read-only file system | Exit 2, Read-only file system |
| Config unchanged after direct refusal | Yes | Yes |
| Source write | Exit 0, expected host bytes | Exit 0, expected host bytes |
| Alias write | Exit 0 | Exit 0 |
| Protected config gained the alias comment | Yes | Yes |
| Protected config retained its inode | Yes | Yes |
| Owner exit after stdin closure | 0 | 0 |

Raw host observations and the probe script were retained at
`/private/tmp/pinfold-cli-alias-probe/result.json` and `probe.py` during
this investigation. They are temporary evidence, not repository fixtures.

## Follow-up boundary

Keep linked-worktree refusal. Plan a repair for the existing protected
directory guarantee first. The plan must account for the programmatic
core's Git-independent mount contract, interactive protection, hardlinks
to other protected configuration, ordinary Git clones with shared objects,
and host mutation assumptions. A scan or link-count check alone is not
proof of enforcement throughout a run. Require real Mac/Linux evidence
for any claimed cross-platform protection and measure any startup scan
before accepting it. Do not introduce a custom filesystem exporter,
case-sensitive storage requirement, new security tier or Git handoff
protocol as part of this bounded repair.
