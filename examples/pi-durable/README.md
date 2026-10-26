# Experimental pi-durable adapter

This example evaluates published `@earendil-works/pi-durable` with Pinfold.
It is a separate Bun program, not a replacement for `pinfold pi` or a complete
`ExecutionEnv`. Its only model tool is `boxed_bash`. Reads, writes and edits use
that tool inside the box. The host owns box creation and command dispatch.

Run these commands from this example directory with Bun 1.4.3 or newer:

```sh
bun install --frozen-lockfile
bun run check
bun run start \
  --pinfold /absolute/path/to/pinfold \
  --image your-existing-image \
  --project /absolute/path/to/project \
  --state /absolute/path/outside/project/job-state \
  --base-url https://your-openai-compatible-endpoint/v1 \
  --model your-model \
  --prompt 'Inspect the project and report a useful next step.'
bun run resume --state /absolute/path/outside/project/job-state
```

Set `PINFOLD_DURABLE_API_KEY` in the host environment when the endpoint needs a
key. The unauthenticated local fixture uses a dummy key. The endpoint must support
streaming OpenAI chat completions and tools. The key is not included in the box
spec or job manifest. Prompts, model answers and tool output are checkpointed;
choose their contents accordingly.

Use the package's `start` and `resume` commands, launched from this
directory, outside the project. The supplied Bun config disables `.env`
loading and automatic dependency installation.

The project must be a git checkout with a `.git` directory. The box works on
a project disk, `durable-<id>` at `/workspace`, filled from the checkout when
the job first starts and kept across recovery. `start` refuses when that disk
already exists, and `resume` refuses when it is missing. The checkout's working
tree is never written. After a completed job, the example fetches the box
repository's branches into the checkout's `.git`, adding objects and
`refs/remotes/durable-<id>/*`; only committed work crosses. Remove the disk
with `pinfold disk rm durable-<id>` once the job is done with it. The example
supplies no box egress, since the host makes model requests. Choose an image
that has a shell, `git` and the project tools you need.

The checkpoint directory, Pinfold and Bun executables, controller and dependencies,
Pinfold's effective state/config/cache and ownership directories, and host PATH
entries must be disjoint from the project. Linux runtime authority is
also checked. Paths and existing symlink ancestors are canonicalized before any
box work, including absent directories under a symlink alias. `from` copies
the project into the box and the fetch writes into its `.git`, so overlap would
expose or alter host state. The box has no mount, so neither checkpoint
database nor writer database is exposed to the guest. Keep the state directory private on the host. Do not copy a live
checkpoint to another path or run it on another host: the namespace is derived
from its canonical path, and recovery refuses a moved namespace. The executable,
project, image, endpoint, model and original prompt are immutable in `job.json`.

Only one controller may write a job. A second writer is refused before touching
the box. After a crash, `resume` waits for the old box's removal before starting
a new box and resuming the harness.

`boxed_bash` is sequential and explicitly `replay: "unsafe"`. An interrupted
command may already have changed the project. The harness reports interruption
instead of automatically repeating that tool execution. This is not exactly-once
execution: a later model turn or caller can still choose a new command. Inspect
partial work before deliberately retrying it. Output is capped by the harness.

SIGINT, SIGTERM and tool cancellation stop the **whole box**, including other
processes in it. There is no per-command kill or promise that independent commands
survive. A hard host-process kill leaves recovery to the next writer and Pinfold's
owner teardown. There is no scheduler, multi-host coordination, automatic retry,
read-only replay tool or general host filesystem tool.

The prototype contract is in
[ARCHITECTURE.md](../../docs/ARCHITECTURE.md#experimental-durable-caller).
Exact dependency versions live in `package.json` and `bun.lock` and move
with pinfold's release pins, including incompatible versions. Any
prototype or dependency change requires `bun run check` and the
standalone regression, guarantee 35, validated on macOS and Linux x64.
The command needs exclusive ownership of the runtime and
an existing image with `sh`, `tail`, `grep`, `tr`, `mkfifo` and `git`:

```sh
bun durable_recovery_preserves_boxed_execution.ts --pinfold /absolute/path/to/pinfold --image your-existing-image
```

It uses a host HTTP fixture and real boxes.
