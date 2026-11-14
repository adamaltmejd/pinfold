# Agents under read-only Git on macOS

2026-11-14. Ticket #298, project-box probe 7. Source `277b436`.
This extends the earlier mount, worktree and concurrency probes. It does
not rerun their attacks or change the spec. No product code, profile,
dependency, test, workflow or release changed. No real login was used.

Commands, fixture implementation and selected raw output are in
[the evidence directory](2026-11-14-agents-readonly-git-probe/).
The `results.txt` output contains labelled excerpts, not complete traces. Reproduction:

```sh
RUSTC_WRAPPER= cargo build --locked
python3 docs/archive/2026-11-14-agents-readonly-git-probe/probe.py MODE
# MODE: inspect, git, nomodel, fixture, interactive, stale, timing
cargo fmt --check
RUSTC_WRAPPER= cargo clippy --all-targets --locked -- -D warnings
```

Each invocation creates a fresh short scratch root under `/tmp`. Do not
reuse an existing root. Full traces and request bodies stay in scratch;
the committed excerpts retain the observed errors and instruction markers.

## Setup and boundary

macOS 27.2.1, arm64, Apple container 1.7.1. Host Git 2.56.1; guest Git
2.47.3. Image `localhost/pinfold/profile-probe-agents:latest`, id
`7b57a746b152c8907a70469f9fa2c7e8eba2783bf6fc983bcd587d1420f23fb3`.
Image agents: pi 1.5.2, Claude 2.1.325, Codex 0.169.0.
The image was reused, never rebuilt or removed.

The clone's `cargo build --locked` passed. The resulting binary reports
`pinfold 0.5.5` under isolated XDG configuration/state, with maintenance
stamped before invocation and update checks disabled. The real cache root
was retained. Current pinfold's host mounts are read-only. The writable
share setup therefore uses Apple `container` directly, as the earlier
probes did. This is not validation of an implemented pinfold share feature.

The commands mount a fresh host project at `/work` writable, and its
`.git` at `/work/.git` read-only. A separate scratch mount carries fresh
homes, trace output and fixture configuration. Its alias of the project's
`.git` is also mounted read-only. Additional Git cases each have their own
read-only `.git` mount. The guest runs as uid:gid 501:20, with no network
interface, a read-only root, tmpfs `/tmp` and an empty capability bounding
set. No existing home, credential file, SSH agent or login store enters it.
Global/system Git config is disabled; safe.directory is explicitly set
as in the earlier probes. Authors are fixture identities.

The host serves fixture HTTP over a fresh Unix socket. Apple's `--ssh`
forwards that socket instead of a real SSH agent. One setup-only root
exec makes its guest socket connectable; agents run without capabilities.
A small guest Node bridge exposes it on loopback port 18080. No external
endpoint is permitted. This isolates harness behavior from proxy behavior;
it does not retest pinfold's production proxy or the earlier npm-pi CONNECT
route failure.

## Git commands

See results section G. Each case uses a separate repository with one initial
commit, modified tracked `a`, and an untracked file. The commit case has
`a` staged by the host. Checkout targets another existing branch. Fetch
has a local origin. Garbage collection is explicit `git gc`.

| Guest command | Exit | Output that matters |
|---|---:|---|
| `git status` | 0 | Correct modified and untracked files; no warning |
| `git add a untracked` | 128 | `index.lock`: Read-only file system |
| `git commit -m fixture` | 128 | `index.lock`: Read-only file system |
| `git stash push -u` | 1 | `error: could not write index` |
| `git checkout other` | 128 | `index.lock`: Read-only file system |
| `git fetch origin` | 255 | `cannot open '.git/FETCH_HEAD': Read-only file system` |
| `git gc` | 128 | `gc.pid.lock`: Read-only file system |

Before/after host inventories compare every regular file's path and
content hash, separately for the worktree and `.git`. All seven have empty
deltas. None leaves a lock or another regular file behind. This does not
claim unchanged timestamps or directory metadata. A separate status trace
shows `openat(...index.lock, O_RDWR|O_CREAT|O_EXCL...) = -1 EROFS`, while
status still succeeds. Optional index refresh can fail silently.

## Agents without a model

Results section N records startup through a noninteractive session's first
request. The host fixture returns HTTP 400, `fixture: no model`, for every
request. No inference or tool call is supplied. All three print an API
failure and exit 1. Claude makes six requests, including its fallback with
a different advertised tool list. Pi and Codex each make one.

The traces cover file/process syscalls and child processes, not just the
agent's own process. They show:

- Pi print mode starts and loads instructions without executing Git.
- Claude reads config, repository identity, recent log, whether a settings path is tracked
  and status. Its log and status calls use `--no-optional-locks`; status
  also uses `--short --ignore-submodules=dirty`.
- Codex runs rev-parse, remote/config queries and
  `status --porcelain --ignore-submodules=dirty`. It also attempts
  `ls-remote https://github.com/openai/plugins.git HEAD`; the guest has
  no external route. That is a harness background action, not a fixture tool.

No index-lock attempt or Git metadata write was observed in these
no-model print sessions. No Git lock remained, and host status was clean.
These are bounded observations of these versions and flags, not claims
about every agent mode, extension, hook or background task.

Interactive sessions are also traced in section I. Each observation has
12 seconds before TERM and a three-second KILL grace. Pi reaches the
fixture error and reads `.git/HEAD` directly for its UI without spawning
Git. Claude and Codex reach their fixture errors after their fresh-folder
or fixture-key prompts are accepted. Claude also reads file lists and a
1000-entry changed-file log. It attempts a shallow clone of
`anthropics/claude-plugins-official` into its fresh home and creates Git
metadata there before the network attempt fails. That is separate from
the shared project. Codex again attempts its plugin ls-remote. No project
metadata write or index-lock attempt appears in these interactive traces;
no project Git lock remains. The controller ends all three with exit 137,
which is an observation limit, not an agent failure code.

## Failed write delivered to the fixture model

See results section F. Each fixture advertises a model through the agent's
supported custom endpoint configuration. Pi uses Anthropic Messages with
a custom models.json provider; Claude uses ANTHROPIC_BASE_URL; Codex uses
a custom provider with Responses and requires_openai_auth=false. Codex's
configuration follows the [official custom-provider documentation](https://learn.chatgpt.com/docs/config-file/config-advanced).
Only fixture placeholders are used.

The first response issues exactly one shell tool call:
`git commit -m fixture`. The second request contains the result; the
fixture then returns a final text response. All three exchanges finish
with agent exit 0. Each trace shows the commit's failed index-lock open.
No lock remains, and the host worktree is clean.

| Harness | What its second model request contains |
|---|---|
| Pi `bash` | `tool_result`, `is_error: true`, the fatal error and `Command exited with code 128` |
| Claude `Bash` | `tool_result`, `is_error: true`, `Exit code 128` and the fatal error |
| Codex `exec_command` | `function_call_output` text with `Process exited with code 128` and the fatal error; its user event also has `status: failed`, `exit_code: 128` |

Thus the model receives the failure. Overall agent exit 0 does not mean
its Git command succeeded. No real model was asked to interpret or obey
instructions, recover, retry, or request a host commit.

## Instructions and the existing profile

The fixture text is:

> This project is a writable host share. Its .git directory is read-only.
> Edit working-tree files only. Only the host stages, commits, stashes,
> switches branches, fetches, and runs Git maintenance. Report requested
> Git writes to the host. After a host commit, Git reads may be stale for
> about a second; a failed read is not evidence that history is absent.

Distinct markers accompany the same text in each file. The captured
requests establish what reaches the model, beyond file access alone:

| Agent | Markers in request | Supplied file / option |
|---|---|---|
| Pi | PROJECT_AGENTS, PI_APPEND | `/work/AGENTS.md`, `/work/.pi/APPEND_SYSTEM.md`; project approved explicitly |
| Claude | PROJECT_CLAUDE, CLAUDE_APPEND | `/work/CLAUDE.md`, `--append-system-prompt-file /probe/append.txt` |
| Codex | PROJECT_AGENTS, CODEX_HOME_AGENTS | `/work/AGENTS.md`, fresh `$CODEX_HOME/AGENTS.md` |

Pi selects AGENTS.md when CLAUDE.md is also present. Claude probes
AGENTS.md but does not include its marker alongside this CLAUDE.md.
That does not establish its behavior when CLAUDE.md is absent. The trace
also probes Claude's managed `/etc/claude-code/CLAUDE.md` and fresh user
CLAUDE.md, which are absent. Pi's installed resource-loader names global
and project SYSTEM.md and APPEND_SYSTEM.md; only the project addition was
exercised. Replacement system prompts and every precedence combination
were not run. Codex system-prompt replacement was not needed or tested.

Current `profile/share/pi/extensions/operating-context.ts` instead says
`commit your work here; the user runs pinfold fetch`. It injects this via
before_agent_start/systemPromptOptions.sections.pinfold. That is the
current disk contract, not suitable share guidance. The probe does not
install or change a production profile. The owner must decide the share
instruction delivery when that implementation is admitted. Instruction
text is guidance; the read-only mount is the write restriction.

## Host commit visibility

Results section S: five trials in one running box. Before each host commit,
the guest primes status/log. The host changes `a`, adds it and commits.
Then the driver polls guest `git status --porcelain; git log -1 --format=%s`
until both agree with the host, with no fixed settling sleep.

Three first observations falsely list every tracked path as untracked
and say the branch has no commits. Two first observations are already
correct. First correct samples complete at 1.038, 0.774, 0.997, 0.701 and
0.943 seconds after the respective host commit. In the last trial, the
read is still wrong at 0.873 seconds. Each recovered without restarting
or changing mounts. These times include exec overhead and bracket sampled
visibility; they are not a guaranteed TTL or precise transition time.

## Large-repository timing

Results section T: 100,000 generated two-byte tracked files in 100 directories,
plus initial `a`. Same repository and command, `git status --porcelain`,
with the ordinary untracked scan enabled. Four sequential samples on the
host, then four in the box. Guest timing is measured around Git inside
Node, excluding container exec startup. Host timing uses Python around
Git. No cache flush. These are observed first/repeated samples, not a
cold-cache benchmark. The host has 10 CPUs; the guest is assigned four.

| Side | Seconds, in order |
|---|---|
| Host | 0.332176, 0.188614, 0.202267, 0.178144 |
| Box | 21.052756, 13.917773, 12.727880, 14.215537 |

All eight statuses exit 0 with empty output. Read-only Git is usable here,
but repeated status scans are expensive on this share. This run does not
separate virtiofs cost, different Git versions, CPU limits or inability
to persist an index refresh. No caching workaround is established.

## Failures, checks and retained state

Initial setup attempts are not outcome evidence: without safe.directory,
Git refused the share as dubious ownership. A socket visible through
virtiofs alone did not reach the host listener. The corrected runs use
an explicit forwarded fixture socket. Initial interactive timeout wrappers
suspended agents with SIGTTOU or stopped at fresh trust/key prompts. The
retained interactive result uses foreground timeout, fixed fixture trust
configuration and explicit fixture-only prompt acceptance. The inspection
command's unmatched config glob exits 2 after printing the useful loader
lines; this is recorded, not treated as a successful full source search.

Build, formatting, Clippy, Python parsing and diff whitespace checks pass.
No test was added or run. Host fsck results are retained separately in
checks.txt. The normal and interactive sessions use fresh homes; their
state writes are outside the project. This probe does not establish
interactive sessions over hours, every agent feature, real-model behavior,
real credentials, Linux, a production share profile or stale-read repair.

The owner decisions are the future share profile's instruction delivery
and whether the observed large-repository status cost is acceptable. The
current disk instruction must not be carried unchanged into that profile.
No specification or behavior change was made to answer either decision.

Every created container is stopped and deleted. No managed disk, image or
volume was created. The existing probe-agents image is retained; no new
image exists to list. The shared container service is left running. Full
diagnostic output remains only in the probe's scratch directories; the
100k-file fixture can be deleted without losing the committed evidence.
The record and its evidence are committed on the existing branch. Nothing
was pushed and no PR was opened. BRIEF.md remains untracked.
