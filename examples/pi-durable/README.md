# Experimental pi-durable adapter

This example evaluates published `@earendil-works/pi-durable` 1.0.4 with Pinfold.
It is a separate Node program, not a replacement for `pinfold pi` or a complete
`ExecutionEnv`. Its only model tool is `boxed_bash`. Reads, writes and edits use
that tool inside the box. The host owns box creation and command dispatch.

Install with Bun and run with Node 26 or newer:

```sh
bun install --frozen-lockfile
bun run check
node prototype.ts start \
  --pinfold /absolute/path/to/pinfold \
  --image your-existing-image \
  --project /absolute/path/to/project \
  --state /absolute/path/outside/project/job-state \
  --base-url https://your-openai-compatible-endpoint/v1 \
  --model your-model \
  --prompt 'Inspect the project and report a useful next step.'
node prototype.ts resume --state /absolute/path/outside/project/job-state
```

Set `PINFOLD_DURABLE_API_KEY` in the host environment when the endpoint needs a
key. The unauthenticated local fixture uses a dummy key. The endpoint must support
streaming OpenAI chat completions and tools. The key is not included in the box
spec or job manifest. Prompts, model answers and tool output are checkpointed;
choose their contents accordingly.

Prepare real `.git`, `.vscode`, `.claude` and `.idea` directories in the project
before starting. The example mounts those paths read-only and the remaining
project read-write. It refuses symlinked protected directories. It supplies no
box egress, since the host makes model requests. Choose an image that has a shell
and the project tools you need.

The checkpoint directory, Pinfold and Node executables, controller and dependencies,
Pinfold's effective state/config/cache and ownership directories, and host PATH
entries must be disjoint from the writable project. Linux runtime authority is
also checked. Paths and existing symlink ancestors are canonicalized before any
box work, including absent directories under a symlink alias. A replaceable
symlink inside the project is rejected even if its target is outside. This
includes the script entrypoint used to launch the controller. Only the
project and protected subdirectories are guest mounts, so neither checkpoint
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

The dependency versions and lockfile use the published 1.0.4 API, including the
real HTTP provider and Node SQLite checkpoint backend. `bunfig.toml` makes a local
release-age exception only for the four pinned Earendil release-family packages:
`pi-durable`, `pi-ai`, `chord` and transitive `pi-telemetry`. These pins are
independent of Pinfold's Pi harness. Other publishers retain the installation age
policy; no global Bun configuration is changed.

The prototype contract and pin-maintenance rule are in
[ARCHITECTURE.md](../../docs/ARCHITECTURE.md#experimental-durable-caller).
The standalone regression is guarantee 35, validated on macOS and Linux x64
and arm64. The command needs exclusive ownership of the runtime and
an existing image with `sh`, `tail`, `grep` and `tr`:

```sh
node durable_recovery_preserves_boxed_execution.ts --pinfold /absolute/path/to/pinfold --image your-existing-image
```

It uses a host HTTP fixture and real boxes.
