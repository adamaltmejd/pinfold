# pinfold architecture

pinfold runs a coding agent in a disposable box: a micro-VM, Apple
`container` on macOS and Firecracker on Linux. The box sees its disks and
declared host mounts. Apple shares expose host files writable, Git
metadata included; other host mounts are read-only. Network egress uses
the box's allowlisting proxy.
Disk-project branches cross through the project's Git remote, `box`;
explicit disk export copies user-requested untrusted files into a new
directory.

## Terms

**Box**: one disposable micro-VM, described by a box spec and
owned by one foreground `box up` process or one detached project owner.
_Avoid_: sandbox, container, VM, run

**Session**: one project command's client connection. A temporary crossing
connection also retains the project box.

**Box spec**: the JSON a caller gives `box up`. It is the whole box: nothing
is added that the spec does not name.

**Project**: a main Git checkout’s canonical root, including its linked
worktrees beneath that root, else the canonical invoking folder. A linked
worktree is admitted only when its common Git directory is that main root’s
`.git`. External linked worktrees and separate Git directories are
refused. A moved or renamed project has a new identity and no configuration.
_Avoid_: repo, workspace

**Profile**: data that shapes a box: an optional image recipe, seeds,
read-only shared files, and config defaults.

**Project home**: a project's `$HOME`, a disk. It belongs to the project
from its first start.

**Disk**: a named ext4 image pinfold keeps for boxes. It outlives the
boxes that attach it; the host never mounts or reads it.
_Avoid_: volume, workspace

**Seed**: a profile file copied into a project home only when missing.
Deleting it there reseeds it at the next start.

## Layers

| Layer | Owns | Used by |
|---|---|---|
| core | runtimes, box lifecycle, transport, proxy, images, profiles, pinned artifacts, maintenance | programmatic callers, pi layer |
| pi | project commands, per-project state, disks and config, herdr | the `pinfold` CLI |
| profiles | box defaults, an optional image recipe, pi's user-level config | applied as data |

Core knows nothing of pi. It owns branch crossing (Disks). To core,
a profile's pi config is files.
Core never reads a profile's `pinfold.toml`; the pi layer does, when it
writes a box spec. Profiles hold no code except pi packages.

## Threat model

The primary threat is an agent reaching data available to the host through
its files, credentials or network position. Prompt injection can turn the
agent against the user. Hostile models are also untrusted. A share
deliberately grants access to selected host files.

| Control | Effect |
|---|---|
| No network | No interface but loopback. One socket to the box's own proxy (vsock on Linux). |
| Egress proxy | Default deny, allowlist by name, every decision logged. |
| Kernel boundary | A VM per box, with its own guest kernel. |
| No privilege | Box work runs as the host uid with an empty capability bounding set; read-only root, no setuid files. On Linux only the guest's PID 1 stays root (Always applied). |
| VMM jail | On Linux the VMM runs as the user in a jail the box owner builds: it reaches only its own VM's files. |
| Host files | Direct guest filesystem access reaches only declared mounts and disks. A writable share exposes its host files, Git metadata included, to modification. Share admission rejects the sources and destinations specified under Mounts. Disk crossing and explicit export retain their separate rules. |
| Profiles and configuration | Directory admission excludes protected configuration and profile paths from projects and shares (Configuration, Mounts). It does not exclude allowed hardlinks from shared files to outside configuration or profile files. |

Not protected:
- A share exposes all its readable contents, including ignored and untracked
  files, Git history and repository configuration. Allowed model endpoints
  can receive those contents. Working files have no protection against
  deletion, corruption or concurrent edits. There is no conflict detection
  or atomic snapshot. Reads may be stale or internally inconsistent. Box Git
  reads can fail after host Git writes. No fixed visibility or recovery delay
  is guaranteed. A failed write may still damage a file. A share file
  hard-linked to a file outside the share, Git objects from a local clone
  included, can be changed in place through the share.
- Host execution or interpretation of box-written files is outside
  containment. This includes hooks and hook managers, build and package
  scripts, editor tasks and extensions, shell integrations, and Git
  configuration and attributes read by host tools. Attributes can select
  filters already configured on the host. The box writes Git hooks and
  configuration that run on the host at the user's next Git command.
  Matching ownership is no defense because box work uses the host uid.
- Host and box Git in one share. A host Git command that rewrites the index
  (`git status`, `git diff`, `git add`, `git commit`) while the box uses Git
  can make a box commit record other files as deleted and exit 0.
  `GIT_OPTIONAL_LOCKS=0` stops host `git status` from rewriting the index.
  Host `git diff` still rewrites it, and host commits while the box commits
  still drop commits from the branch and record files as deleted. A box commit within
  about a second of a host commit can drop the host commit from the branch.
  Lost commits survive only in the reflog or as dangling objects, if at all.
- There is no cross-box coordination. A box-created nested `.git` can change
  the project selected by discovery, including a linked worktree’s selected
  main root, and cause a missing-configuration refusal.
- Symlinks resolve in the guest namespace and do not grant access to an
  otherwise unmounted host path; they can reach other paths already
  available in the box. Host tools following box-written symlinks can reach
  outside the share. Checking a file before using it is not an execution
  boundary: another process can replace it between the check and use.
  Host creation of aliases or replacement of admitted mount sources during
  a run is outside the admission guarantee.
- Project contents: an agent with a model API can put them in a prompt.
- What the host does with fetched refs: merging, checking out, building
  or opening them runs the box's content as the user. A merged
  `.husky/pre-commit`, editor config or planted `sub/.git`
  is the user's from then on.
- The Git remote's guarantee ends at refs under `refs/remotes/box/*`, and
  Git writes them:
  - A refspec on the command line (`git fetch box main:other`) and
    `git pull box` write the user's own branches, and `git pull` checks out
    box content.
  - With `--prune` or `fetch.prune`, the box deletes `box/*` branches by
    deleting its own; `fetch.pruneTags` with prune deletes the user's
    tags, as the box lists none.
  - A box branch named `HEAD` lands as `refs/remotes/box/HEAD`, so `box`
    names a commit the box chose.
  - A fetch may update some `box/*` refs and fail others, such as `x/y`
    beside an existing `box/x`. `--atomic` moves none, but leaves the
    checked objects in the repository.
  - Git's own fetch behaviour applies: it writes `FETCH_HEAD`, may run
    auto maintenance, and may fetch submodule commits the box's history
    names from the submodules' own remotes.
  - `git push box` moves `refs/remotes/box/BRANCH` to the pushed commit
    until the next fetch.
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token, registries).
- A login route gives every process in the box the host login's authority
  over every path at the selected origin. It does not restrict provider
  operations, inspect request bodies, or prevent an upstream from returning
  credentials in its response.
- Configuration is keyed by the project id: the folder name, sanitised and
  cut to 32 bytes, and 12 hex digits of the canonical path's hash. Another
  repository placed at that path gets its configuration.
- The Rex relay (pi layer, Rex) lets the box's Rex server reach the Mac Rex
  app. Accepted: macOS notifications the guest raises;
  clipboard writes while the guest's window is in front; following a link
  from the box, where a click on a loopback link makes the app forward a
  Mac loopback port into the box and open a guest-served page in the Mac
  browser, outside the box proxy; and Rex's own enforcement, which can
  change with a Rex build or a setting changed after the host is added.
- Image recipes are caller-trusted software. Apple builds have unrestricted
  egress. Linux builds use the separate image-preparation role below.

## Runtimes

One runtime per OS, not configurable:

| Host | Runtime | Kernel |
|---|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) | one VM per box |
| Linux | Firecracker, pinned (Pinned artifacts), run as the user in pinfold's VMM jail | one VM per box, pinned guest kernel |

Anything else is refused: podman or docker as a box runtime, Firecracker
under its own jailer or outside pinfold's VMM jail. Nothing warns at run
time; `doctor` reports the runtime. Image operations use Apple `container`
on macOS and a pinned BuildKit builder/converter on Firecracker on Linux,
independently of the box execution interface. Common core owns image retention and in-use protection.
Execution backends own native disk creation and ownership mechanics. Common
core owns disk claims, exclusivity, recovery and cleanup policy.
Backends complete guest initialization through readiness. The common box
owner holds the child, proxy and cancellation and decides cleanup.
Cancellation reaps preparation helpers and joins startup, proxy and diagnostic
workers before completed teardown. Host name lookup has a ten-second deadline.
At most 64 resolver workers may be outstanding per process; a full resolver
budget fails as a lookup timeout does. A slot remains held until its worker
exits. Cancellation discards its result without dialing. An abandoned lookup
holds nothing of pinfold's: no credential, socket, lock or policy reference.
Teardown may finish without joining a lookup still in the system resolver;
all other workers are joined. Disk preparation helper removal and disk undo
each have a 300-second deadline. Native runtime removal has a 300-second deadline; expiry cancels and reaps its helpers
and reports `cleanup-timeout`. Linux unmount has a 30-second absolute deadline.

Preflight refuses box startup on older, prerelease or unrecognized runtime
versions. Inventory and cleanup remain available on older versions.

Linux is x64 only. Preflight requires a user other than root, `/dev/kvm`
readable and writable by the user (the `kvm` group), unprivileged user
namespaces, Landlock ABI 5 or later (Linux 6.10+), `mkfs.ext4`
(e2fsprogs). Without user
namespaces or Landlock ABI 5, `up` is refused as `runtime`, naming the missing requirement's token
(below); there is no unjailed fallback.

Where AppArmor restricts unprivileged user namespaces
(`kernel.apparmor_restrict_unprivileged_userns=1`, Ubuntu 24.04+'s
default), the jail needs pinfold's AppArmor profile. It is unconfined
except that it grants `userns`, and it attaches to pinfold's installed
executable path. `pinfold apparmor-profile` prints it; loading it needs
root once, and pinfold never runs as root. Without it, `up` is refused
as `runtime` with its own token, `userns-apparmor`, chosen from the
sysctl rather than the failing call.

## Box

### Lifecycle

1. Claim the name with a per-user lifetime lock, independent of XDG and
   `HOME`. Hold it through runtime removal, child reaping and state cleanup.
   Record a fresh generation and state directory; label the box
   with that generation. Claim acquisition waits up to ten seconds through
   contention, including maintenance probes, and is signal-cancellable.
   A name still held at the deadline is `name-in-use`; abandoned partial
   claims are reclaimable. Nothing (seeds, disks, proxy, box) is created
   before the claim.
2. Claim the box's disks (Disks), creating any that are new. A new
   disk's `from` is size-checked here. Record writable disks unclean and their
   attachments before any preparation write. Remove only an empty
   formatter-created `lost+found` on a fresh disk before population or guest
   work; reused disks retain their contents. A fresh disk is populated from
   its initial source first, as the box user. Reused disks are never populated
   again. On Linux each fresh disk is populated immediately after its own
   mount, before deeper attachments. Then create missing generated-mount parents, and only then attach
   generated configuration. Preparation never marks a disk clean. On Linux a
   failed start may retain a reused disk’s unclean mark before BOOT even when
   no disk was written. An ordinary
   directory population already created is retained; a symlink or non-directory
   anywhere along the generated path fails startup. Before attaching generated
   configuration below a writable disk, pinfold creates missing parent directories as the
   box user, without following symlinks or changing existing ownership.
   On Apple this uses a transient networkless preparation box under the disk
   claim. Preparation ends before ordinary box work starts. The helper owns
   only fresh disk roots with `CAP_CHOWN`, then drops groups, gid, uid and
   effective/permitted capabilities before descriptor-relative no-follow
   directory creation. Its transient `CAP_SETUID` and `CAP_SETGID` permit
   that drop. Ordinary boxes gain no capability. The helper carries the box's
   name, owner and generation labels, so `clean` removes a leftover.
3. Runtime sockets use a checked private generation directory under the
   host ownership directory, independently of state-root length. Session and
   shutdown sockets remain host-only and outside the VMM's allowed socket directory.
4. Run the box with no network interface and the socket carried in
   (Transport). PID 1 is `pinfold init`. On Linux each mount reaches the
   guest as a read-only ext4 image written from the host directory at this
   start, so a host change after it never appears in the box.
5. `pinfold init` relays `127.0.0.1:3128` to the socket and reports ready
   (on Linux its exec service does the relay, exec and ready report). On
   Linux it mounts the disks first; on Apple the runtime does. On Linux
   PID 1 makes each mount point without following a symlink: a symlink a
   box left on a disk fails the start, and PID 1 creates nothing through
   it. Disks attach only here, at boot; nothing attaches or detaches one
   later. On Linux PID 1 then puts box work in Always applied's state. Before
   ready, the box copies the profile's missing home seeds as the box user.
6. Apple only: wait until the runtime lists the box running, including
   without egress. When egress is present, one transient root invocation of
   the supplied init changes only the fixed forwarded socket's permissions;
   it executes no image program and inspects no share content. The operation
   uses a fixed cwd, no environment lookup and no share traversal, and
   validates the socket without following a symlink.
7. `exec` work runs with `HTTPS_PROXY` and `http_proxy` set to
   `http://127.0.0.1:3128`.
8. Sync and unmount the disks (Linux: init, after ending box work with
   the work cgroup's `cgroup.kill`; Apple: the runtime's stop),
   stop and remove the box, record the disks clean and release them, and
   delete the socket.

The box owner does steps 1–6, holds the proxy, and does step 8 on shutdown.
Public `box exec` and project commands are admitted only after steps 1–6
complete, including the fixed root socket operation. No caller command runs
before that boundary. Public `pinfold box up` ends on `down`, stdin EOF or
SIGTERM. If the owner dies, the
socket dies and the box has no way out (fails closed); `box prune` removes
leftovers host-wide. A detached project owner uses the same lifecycle.

### Process interface

```
pinfold box up < spec.json        # prints ready, holds the box, ends with down
pinfold box exec BOX [--tty] -- argv…   # stdio through, exit code back
pinfold box stat BOX              # one JSON object of the box's memory, pids and OOM kills
pinfold box down BOX              # asks its owner to tear down; an absent box exits 0
pinfold box list --label KEY[=VALUE]…      # JSON lines; repeat --label to AND filters
pinfold box prune                 # remove boxes whose `up` is gone, print each removed
pinfold box inspect RUN           # one JSON object: a run's retained records
pinfold box events RUN [--limit N] [--after CURSOR]   # one JSON object: a page of a run's egress log
pinfold disk list [--json]        # managed disks and their recorded origins
pinfold disk rm NAME|--all [--json] # unattached disks in this state root
pinfold disk export NAME DEST [--image IMAGE] [--json] # untrusted files into a new directory
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache] [--allow HOST]… [--json]
pinfold image rm NAME [--json]
```

`image build` builds a caller image (see Images). Build progress goes to stderr.
The default report names the resulting image. With `--json`, image operations
print the following records. A successful build exits 0:

```json
{"event":"built","image":NAME,"ref":"localhost/pinfold/image-<NAME>:<build>","latest":"localhost/pinfold/image-<NAME>:latest","id":ID,"labels":{…},"base":DIGEST|null}
```

On Linux `id` is the runnable OCI manifest SHA-256; converter recipe,
bootstrap identity and raw-root hash are separate. On Apple `id` is the image's runtime id, the same value a box's `ready` and `list`
report as `image.id`; it is null when the runtime cannot resolve the built
ref after a successful build. `labels` is every label the build put on the
image. A failed build exits 1 with the last 40 lines of the runtime's
build output, bounded to 64 KiB even for an unterminated line:

```json
{"event":"failed","image":NAME,"log":[LINE,…]}
```

A refusal before the build (a bad name or argument, a missing context or
Containerfile, a `dev.pinfold.` label, a missing runtime) exits 1 and
builds nothing; `image` is null when no name was given:

```json
{"event":"refused","image":NAME,"reason":"spec"|"runtime","detail":TEXT}
```

`image rm` (see Maintenance) exits 0 with removed and in-use ids;
a bad name is refused as `spec` like a build's:

```json
{"event":"removed","image":NAME,"ids":[ID,…],"in_use":[ID,…]}
```

On an absent box, `exec` exits 3 with `pinfold box exec: no box named ...`;
any other exit code is the command's. It exits 3 without running a caller
command until startup completes. Agent executables come from the image and launch through PATH.

`stat` prints one JSON object with every key present:

```json
{"box":NAME,"oom_kills":N|null,"memory":{"current":BYTES|null,"peak":BYTES|null,"limit":BYTES|null},"pids":{"current":N|null,"limit":N|null}}
```

On Linux every field comes from the guest kernel, through init; the
memory limit is `memory`. Apple answers the limits from what it reports for the box and null
for the rest. A field the runtime cannot answer is null. `oom_kills` is
monotonic for the box's life; the caller keeps its own baseline. On an
absent box, `stat` exits 3 with `pinfold box stat: no box named ...`, like
`exec`.

`up` prints one `ready` line once the box accepts `exec`:

```json
{"event":"ready","box":NAME,"run_id":ID,"owner":PID,"labels":{…},"image":{"id":ID,"ref":REF},"egress_log":PATH|null}
```

`run_id` is the box's generation, the value of its `dev.pinfold.generation`
label: 64 lowercase hex characters, new at every `up`. It names the run's
records (Run records). `owner` is the `box up` process; `labels` is the box's full label set, as
the runtime reports them, image labels included. `image.id` is the image the
runtime resolved `image` to; `image.ref` is the reference as the spec (or its
profile) gave it. `list` spells `ref` as the runtime records it, so a
caller compares images by `id`. At `up`, the image's
`dev.pinfold.*` identity labels are copied onto the box, unless pinfold set
that label itself (a project box's `dev.pinfold.project`). `egress_log` is the
absolute path of the box's egress log (see Egress proxy), null without
`egress`.

The caller keeps `up`'s stdin open for the life of the box; closing it is
`down`. The stream ends with one `down` line after teardown:

```json
{"event":"down","box":NAME,"run_id":ID|null,"reason":REASON}
```

`REASON` is one of `stdin-closed`, `signal` (SIGTERM, SIGINT or an accepted
`box down` request), `audit-log` (an egress log append failed or would exceed
its 128 MiB cap), or `exited` (the box's init ended on its own;
`detail` carries its exit status). `up` exits 0 for `stdin-closed` and
`signal`, and 1 for `exited` or `audit-log`; `box` is null when a signal ended `up` before
its spec parsed, and `run_id` is null when it ended `up` before `ready`.

When `up` refuses, it prints one JSON line instead of `ready` and exits 1:

```json
{"event":"refused","box":NAME,"reason":REASON,"detail":TEXT}
```

`REASON` is `spec`, `profile`, `runtime`, `image-missing`, `name-in-use`,
`login`, `disk-in-use`, `disk-unclean`, `runtime-path` or `limit` (see Disks);
`box` is the `name` of stdin's first JSON value when that is an object with
a string `name`, else null.
Early refusals create no box, box state, disk, seeds or extracted profile
resources. `disk-in-use`, `disk-unclean` and `limit` are decided after
the claim; they release it and leave no state. Failed-start cleanup removes
claimed state and any disk this `up` created when no runtime was created or
its removal is confirmed, subject to the disk deletion checks below. If stop
or removal cannot confirm an existing runtime is gone, the claim, state and
disks remain for checked recovery. Fetched artifacts and ordinary home seeds
may already exist and are not rolled back.
Stable coordination lock files may remain. `up` asks the runtime to resolve `image`, so any reference the
runtime resolves locally is accepted; `image-missing` means the runtime could
not.

When `up` fails after it began creating, it removes what it made, prints one
`failed` line instead of `ready` or `down`, and exits 1:

```json
{"event":"failed","box":NAME,"detail":TEXT}
```

Every `up` ends its stdout with exactly one of `refused`, `failed` or
`down`.
A tag resolving to a different image during startup fails with
`image-changed` in `detail` before `ready`.

`down` uses a host-only control socket and addresses
the observed generation. Public requests retain the 64-byte generation and
`done`, `gone` or `failed` reply. Private project requests prefix that generation
with `P` and receive `stopping` after validation, before the final reply. The
owner acknowledges completion after teardown. A timed-out request
fails without releasing ownership or removing a live owner's state.
Orphan removal and `prune` acquire the lifetime lock and recheck the
generation before removing anything; an old observation cannot remove a
replacement. Control sockets never enter boxes. The checked per-user lock
directory is `/private/tmp/pinfold-<uid>` on macOS and
`/tmp/pinfold-<uid>` on Linux. Persistent lock files are never unlinked.

`list` prints one JSON line per pinfold box whose labels match every `--label`.
On Linux a box is a claim whose VMM has started; `list`
reads pinfold's own claims and whether each VMM still runs, and asks no
runtime. On Apple it reads `container list`.
`--label KEY=VALUE` matches that value; `--label KEY` matches any value of
`KEY`; every box carries `dev.pinfold.owner`, so `--label dev.pinfold.owner`
lists them all. A container without it is no box: `exec` and `stat` exit 3
on it and `down` leaves it. Each line carries the box's labels and its
image, then its `dev.pinfold.owner` pid (or null), whether its
generation owns the lifetime lock, and the runtime's RFC
3339 `created` time and `state` (`running`, `stopped` or `unknown`):

```json
{"name":NAME,"labels":{…},"image":{"id":ID,"ref":REF},"owner":PID,"owner_alive":true,"created":RFC3339,"state":"running"}
```

`prune` removes every pinfold box on the host whose `up` is gone and prints one line per
removed box, nothing when there was none:

```json
{"event":"pruned","box":NAME,"owner":PID}
```

The box spec `up` reads from stdin:

```json
{
  "name": "job-3f2a…",
  "labels": { "dev.example.job": "…" },
  "profile": "builder",
  "image": "…",
  "mounts": [{ "host": "/…/fixtures", "guest": "/fixtures" }],
  "disks": [
    { "name": "job-3f2a-work", "guest": "/workspace", "size": "20G", "from": "/…/clone" },
    { "name": "job-3f2a-home", "guest": "/state" },
    { "name": "corpus", "guest": "/corpus", "readonly": true }
  ],
  "env": { "HOME": "/state/home", "GH_TOKEN": { "from": "GH_TOKEN" } },
  "egress": {
    "allow": ["api.github.com"],
    "routes": {
      "api.internal": "127.0.0.1:7777",
      "openrouter": {
        "to": "https://openrouter.ai",
        "headers": { "Authorization": { "from": "OPENROUTER_API_KEY", "prefix": "Bearer " } }
      }
    }
  },
  "cpus": 4,
  "memory": "8G"
}
```

- No `egress`: no way out at all (gates).
- `env` is exact apart from the always-applied values below; nothing is inherited.
  An env name must match
  `[A-Za-z_][A-Za-z0-9_]*`; any other name is refused as `spec`, naming it.
  A literal holding NUL is refused as `spec`, naming only the variable.
  `{ "from": "NAME" }` reads the caller's value. Guest values reach the
  guest after boot, over pinfold's own channel to init (Always applied);
  pinfold passes none to the runtime as environment, so they cannot
  configure the host runtime client. A `from` value never reaches argv,
  the runtime's records or a file pinfold writes. A literal `HOME` is also
  the path that places generated mounts (Pi login), so it appears in
  their mount arguments and the launch record;
  multiline values remain intact. The runtime executable is found on the
  host's `PATH`, never the spec's.
- A label key starting with `dev.pinfold.` is refused as `spec`, naming
  it: the namespace is pinfold's.
- `profile` applies the profile's `home/` and `share/` (see Profiles), and
  its image if `image` is absent; egress, env and resources come only
  from the spec.
- `PINFOLD_ALLOW` defaults to the spec's comma-separated `egress.allow`,
  empty without egress; the spec's own env wins. No per-agent authentication
  env is added at exec. Project commands add Herdr metadata (pi layer).
- Login routes alone generate read-only configuration: Claude at
  `/etc/claude-code/managed-settings.json`, Codex at `/etc/codex/config.toml`,
  and Pi at `$HOME/.pi/agent/extensions/pinfold-login/index.ts`. Each containing
  directory is one mount. Pi mounts only that named subdirectory; sibling
  extensions remain. Pi routes require an absolute plain `HOME` on a writable
  disk, including explicit images without profiles. Generated mount paths
  may not overlap each other or a caller mount, contain a disk destination,
  or sit below a read-only disk. Paths are compared after lexical normalization.

#### Mounts

- A mount's kind is `readonly` by default, or `share`. Read-only mounts
  retain their existing visibility semantics. A share exposes an existing
  canonical host directory writable on Apple. Linux refuses a share
  as `spec` before maintenance, artifact preparation, credential resolution,
  claims or resource creation. A box writes to its disks, tmpfs and share mounts.
  Read-only mounts show the host directory as of `up`; a later host change
  may or may not appear.
- A share mounts the whole directory writable, `.git` included. Share
  sources must be plain directories. Admission resolves their host ancestors
  without following a final source symlink. The supplied init attaches
  read-only after every caller attachment.
- A symlink in the image can make two declared destinations name one guest
  path. This can change which declared source the box sees there. Admission
  does not inspect the image tree.
- Share admission resolves one set of protected host directories: pinfold's
  configuration, state, cache, artifacts, supplied init and ownership
  directories, and the selected profile directories. A share source that
  contains or is contained by any resolved protected directory refuses as
  `spec`, including targets of externally linked directories. Selected
  profile files are protected too. These share checks add no refusal for
  readonly mounts or disks unless the destination is nested inside a share
  and is not reached through plain directories.
- Apple supplies Git an exact-path `safe.directory` exception for each
  share's guest root, never `safe.directory=*`. A share destination that Git
  would interpret as a pattern or otherwise not literally, including a
  trailing `/*`, refuses as `spec`.
- Mount hosts must resolve to existing directories (read-only mounts accept
  symlinks to directories); a host path that cannot be read or is not a directory is
  refused as `spec`, naming it. A mount or disk nested in another applies
  inside it whatever the spec's order; two at one lexical guest path, with
  `.` and `..` resolved, are refused as `spec` before preparation.
- On Linux a box attaches at most 14 disks and mounts, pinfold's own
  (the profile's `share/` and each generated configuration directory) included;
  more are refused as `spec`.
- A disk `name` is validated like a profile name, and named once per
  spec. `size` is a whole number
  followed by `G`, at least `1G`, default `64G`; it caps a disk pinfold
  creates and is ignored for one that exists (`disk list` reports its
  size). A disk is created on first use, empty or filled from `from`.
- `from` is a host directory, checked like a mount host, read only when
  pinfold creates the disk and ignored afterward. Its tree is copied
  without following symlinks: directories, regular files (644 or 755, the
  exec bit kept) and symlinks as links. Other file types are skipped. A
  tree whose file sizes sum past the disk's `size` is refused as `limit`
  before anything is copied. `from` with `readonly` is refused as `spec`.
- A mount or disk guest path at or below `/proc`, `/sys` or `/dev`, after
  resolving `.` and `..`, is refused as `spec`, naming it: the guest
  kernel's filesystems hold the box's controls. So is one at `/`, `/tmp`,
  `/etc` or `/etc/resolv.conf`, which would hide Always applied's tmpfs
  `/tmp` or empty `/etc/resolv.conf`.
- A mount or disk path holding `,` or an ASCII control character is
  refused as `spec`, naming the path.

#### Other validation

- `memory` is a whole number followed by `M` or `G`, at least `256M`;
  anything else is refused as `spec`, naming it. So is a malformed `size`.
- A string route value must be `host:port`, the port a number in
  1–65535; anything else is refused as `spec`, naming the route.
- A suffix entry `.X` in `egress.allow` is refused as `spec`, naming it,
  when X is a public suffix or has one below it (`.com`, `.github.io`,
  `.amazonaws.com`): anyone can get a name there. A `*.` or `!` rule
  counts as the name after it. The list is the Public Suffix List pinned
  in the repository. Exact names are not checked.
- An unknown key at any level of the spec is refused as `spec`, naming the
  key; pinfold never applies a spec partially.

### Run records

A run is one `up` from `ready` to its end. Its records live in
`~/.local/state/pinfold/runs/<run_id>/`, outside the box's state dir, so
they outlast teardown. The directory is 0700 and its files 0600, created
without following symlinks. `refused` and `failed` retain no run.

The launch record is written once, atomically, after startup completes
(Lifecycle step 6) and immediately before `ready`. Failed-start
cleanup removes the run's directory:

```json
{"run_id":ID,"box":NAME,"time":RFC3339,"pinfold":VERSION,"runtime":{"name":"apple"|"firecracker","version":V},"image":{"id":ID,"ref":REF},"profile":NAME|null,"labels":{…},"mounts":[{"host":PATH,"guest":PATH,"kind":"readonly"|"share"}],"disks":[{"name":NAME,"guest":PATH,"readonly":BOOL}],"env":[NAME,…],"egress":{"allow":[…],"routes":{NAME:{"kind":"host-service"|"injecting"|"login","to":TARGET}}}|null,"cpus":N,"memory":M}
```

Every field is a value the box spec carries or that `ready`, `list`,
`doctor --artifacts` or `doctor` already prints. It never holds an env value, a header name,
value or prefix, a login token, argv or command output. `env` lists declared
variable names only; Apple-generated Git configuration names are omitted
unless the caller declared them. A route's `to` is its `host:port` or origin.
The record is what this run applied at start; a later change to config leaves it as it is.
Each recorded mount includes its applied access kind.

The terminal record is written once, after teardown, before the owner
acknowledges `box down` and before `up` prints `down`:

```json
{"run_id":ID,"time":RFC3339,"reason":REASON,"detail":TEXT|null}
```

`reason` and `detail` are the `down` line's. A project owner records
`sessions-ended` with null detail after its last unkept lease ends. Session
argv, output and statuses are not retained. Owner death and orphan pruning
create no terminal record. A failed teardown attempt writes a separate
`teardown.json` with run id, time and reason `teardown-failed`; it does not
assert runtime removal or authorize disk recovery. `box inspect` includes
this record under `teardown` only when present, with the same record states.
An audit failure takes precedence in the final terminal reason and client
notification, including a failure during teardown. Completed command status
is retained.

An egress log append failure or cap exhaustion (Egress proxy) stops a
running box. No other record write failure does. A launch record that cannot be
written fails the start with `failed`, naming it. A terminal record that
cannot be written is reported on stderr; `down`, its reason and the exit
code are unchanged. A run-record write never delays or alters an audit
append.

`inspect` prints:

```json
{"run_id":ID,"launch":{"state":STATE,"record":{…}|null},"terminal":{"state":STATE,"record":{…}|null},"egress_log":{"state":STATE,"path":PATH|null,"bytes":N|null}}
```

`events` prints the egress log's complete lines from `--after`'s position,
oldest first, at most `--limit` lines (1–100, default 100) and 64 KiB:

```json
{"run_id":ID,"state":STATE,"events":[{…},…],"next":CURSOR|null,"more":BOOL}
```

`next` is an opaque cursor after the last returned line; `more` is true
when complete lines remained past it at the read. A line that is not a JSON
object, or exceeds 64 KiB, appears in place as `{"unreadable":true}`. An
incomplete final line is not returned.

`STATE` is `available`, `not-observed` (no terminal record: the run is live,
or its owner died without one), `not-applicable` (no egress), or
`unavailable` (unreadable, malformed, or over 64 KiB). Neither query
reports liveness; `list` does. Run records never enter a box.

Both queries take no lifetime lock, start no runtime client
and read only the run's directory, so they answer with no runtime and while
the runtime stalls. A RUN that is not 64 lowercase hex characters, a
`--limit` out of range, or a cursor from another run exits 1 before any
path is built. A run with no records exits 3 with `pinfold box inspect: no
run ...` (or `events`); it may never have existed or have been removed
(Maintenance).

### Always applied

- No network interface but loopback, a read-only root, a tmpfs `/tmp`.
- All box work runs as the host uid:gid with an empty capability bounding
  set and `no_new_privs`.
- `NODE_USE_ENV_PROXY=1`.
- On Apple, shares add `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_n` and
  `GIT_CONFIG_VALUE_n` for exact guest-root `safe.directory` entries,
  appended to the caller's declared Git configuration.
- Apple adds: `--network none`, `--cap-drop ALL`, `--read-only`. PID 1 runs
  as the host uid. The guest kernel boots with
  `sysctl.user.max_user_namespaces=0`; init requires an effective
  `user.max_user_namespaces` of 0 before readiness. An unreadable or
  nonzero value fails startup. All Apple agent boxes disable nested user
  namespaces. Workloads requiring them are unsupported.
- Firecracker adds: no network device; the root disk and init disk
  read-only; `vcpu_count` and `mem_size_mib` from `cpus` and `memory`, 4 and 1G
  when the spec names none, as Apple's defaults; no swap; the image's
  `Env` as defaults under the spec's `env`; an empty `/etc/resolv.conf`;
  Firecracker's own seccomp filters. The box owner runs the VMM as the user in
  the VMM jail with a parent-death signal, so the VM ends with its owner.
- In the Firecracker guest, PID 1 sets `user.max_user_namespaces` to 0
  and forks the exec service as the host uid with every capability set
  empty, in a root-owned work cgroup that holds the pids cap and that box
  work cannot leave, and keeps only `CAP_SYS_ADMIN` itself. PID 1 then
  only reaps and answers Lifecycle step 8's unmount; it handles no
  workload bytes.
- pinfold puts no secret value in a kernel command line, a VM
  configuration, an image layer, a disk or a persisted request; what a
  guest process writes is its own. Explicit disk export may copy those
  user-requested bytes to a new host directory, never credentials forwarded
  by pinfold or diagnostic payloads. On Linux the spec's env reaches
  the guest only over the exec connection, after boot, and the exec
  service holds it in memory. On Apple `container run` gets none of it:
  the owner writes it to init's stdin, and init reads it before ready,
  holds it in memory and serves it on an abstract unix socket, which has
  no file. Each `init exec` and session exec reads it there and applies it
  over the image's environment, which Apple gives every exec.

On Apple, read-only is a guest mount flag; pinfold does not rely on the
host share server enforcing it. Box work has the host uid:gid, an empty
capability bounding set and `no_new_privs`. The root is read-only and
nested user namespaces are disabled and checked before readiness. These
controls prevent ordinary box processes from removing or replacing the
read-only mount. A guest-kernel compromise is outside this protection.
Every process that looks at a share runs as the host uid.

Runtime command lines and VM configurations are built as data.

### VMM jail

On Linux the box owner builds each VMM's jail itself, with no privileged helper,
before it execs Firecracker:
- new user, mount, PID, network, IPC and UTS namespaces; only the host uid
  and gid map, each to itself;
- `pivot_root` into an empty tmpfs holding only the Firecracker binary,
  the guest kernel, the root and init disks, the box's disks and the
  mounts' images (read-only binds, but for the box's writable disks),
  `/dev/kvm`, the vsock socket directory and the VM configuration file
  (Firecracker runs with `--no-api`); the old root is detached. Session and shutdown
  control sockets and the generation marker stay outside the VMM's allowed
  socket directory;
- a capability bounding set of zero, `no_new_privs`, and the user's
  supplementary groups, which reach nothing in the jail (dropping them
  needs privilege);
- a Landlock ruleset requiring ABI 5 or later; older ABIs refuse before
  ruleset creation. It handles every known access right through the running
  ABI, TCP and `/dev/kvm`'s ioctls included. Newer ABIs run normally; new rights
  stay unhandled. It allows only: read and execute on the binary; read on
  the kernel, the root and init disks, the mounts' images, the box's
  read-only disks and the configuration; read and write on the box's
  writable disks; read, write and ioctl on `/dev/kvm`; making
  and connecting to sockets (ABI 9's `RESOLVE_UNIX`), removing files and
  reading the directory in the vsock socket directory.

A compromised VMM reaches only that tree, as the user: no home, no other
box's disks or socket, no host network or abstract socket, no host
process. Getting further needs a host kernel bug.

### Process supervision

- Always `-i`. Project commands use a TTY when stdin and stdout are both
  terminals; public `box exec --tty` can force one. Host `TERM` and
  `COLORTERM`, when present, override box defaults for that command.
  Host dimensions are sent only when stdout is a terminal and both values
  are nonzero. Redirected stdout sends no size and is not a setup failure.
  Raw mode on a real host terminal is established before dispatch and must
  succeed. Initial guest dimensions are applied once before CMD; a required
  guest terminal ioctl failure fails execution.
- Each project client owns its terminal and command transport.
  Apple project execution carries command frames over the native client's
  private pipes and creates its PTY in the guest. Public Apple TTY exec
  gives the native client a private host PTY, opened close-on-exec with its
  slave raw before spawn. Only pinfold changes the caller's terminal. No native
  Apple client holds a caller-terminal descriptor. Public non-TTY Apple exec
  uses separate pipes. Framed execution keeps separate stdout and stderr channels.
  The box-owning native client's stderr is piped. Startup diagnostics retain
  at most 64 KiB in memory. Before ready, stdout EOF or native-client exit
  starts a one-second failure deadline even if a descendant holds stderr.
  After ready, stderr drains continuously and retains its latest 64 KiB in
  memory, reported only on an unexpected native-client exit. Diagnostics
  are escaped with the shared helper when printed and never written to a
  file. Guest failure replies are escaped before becoming host diagnostics.
  Cancellation and teardown close and join diagnostic drains before
  reaping the client. Bootstrap diagnostics stop at admission without waiting
  for EOF from descendants. The detached owner uses `/` as its working
  directory and retains only configuration roots/selectors, referenced static
  sources and PATH needed by runtime and Git helpers.
  Linux uses pinfold's vsock transport. Framed streams bound queued data with
  consumption credits. Command input cannot block window changes or status.
  Non-TTY execution preserves separate
  byte streams. Closing a non-TTY output consumer closes that guest stream's
  pipe reader so its writer observes SIGPIPE. Public Apple exec closes the native
  pipe so the native client observes SIGPIPE. An observed command status is kept.
  An observed native exec status survives later relay failure or cancellation.
  After native exec exits, remaining output drains to EOF or a one-second
  deadline; a descendant holding an endpoint cannot retain the client.
  Both platforms restore only termios this execution changed. Restoration
  checks that its process group owns the foreground terminal before writing.
  A non-raw execution leaves later terminal settings alone. Handled
  cancellation restores owned changes and prevents late raw setup. Apple
  never changes caller file status flags. Private stdio copiers carry no owner
  authority, exit on client death, and are killed and reaped on cancellation.
  A restoration failure
  fails an otherwise successful command; an existing command failure is
  preserved. This does not cover process-killing signals.
- When a project client departs, its unprivileged guest helper sends TERM
  to that command's process group, then KILL two seconds later if it remains.
  A terminal departure first closes its PTY master, hanging up the guest terminal.
  Client signals, Leave after a signal, lease EOF and control connection loss
  are departures. The helper watches connection loss independently of command
  I/O. Helper setup or relay failure also ends the command group and reaps its
  leader.
  The group comes from the helper's own child, never workload data.
  Other sessions, the helper and relay are untouched. Normal command completion
  leaves background children running, with output delivered for up to one second
  after the command exits. A process that leaves the command's process group,
  or for a terminal session ignores hangup after leaving it, may keep running.
  `pinfold stop` ends everything. TTY Ctrl-C and Ctrl-\ remain terminal input.
  Plain stdin EOF is not departure. Public `box exec` sends no signal on departure.
- Observed command exit codes pass through, including 125, 126 and 127;
  `128+n` for a command's signal death. A project setup failure before CMD
  starts, including cwd, terminal setup or executable lookup, uses 125.
  External box teardown or an accepted shutdown during startup uses 125
  when no command completion was observed. Host stop handling uses `128+n`
  only for a signal received by the pinfold process. An observed command status
  survives leave or teardown failure, which is reported on stderr and in the run
  record. Public process-interface exit codes are unchanged. A completed exec
  result is checked before a ready shutdown or signal. Terminal-restoration
  failure preserves an observed nonzero command status and changes observed
  success to 125.
- Every process started in a box through `exec` starts with `oom_score_adj`
  1000. On Linux, unprivileged work and its descendants may lower it to
  -999, but cannot obtain score-based OOM immunity through either
  `oom_score_adj` or the legacy `oom_adj` interface. The exec service
  remains protected at -1000. This bounds score-based immunity; it does
  not guarantee guest liveness under arbitrary memory pressure.
- After a SIGKILL of pinfold, `box prune` cleans up; the terminal needs
  `stty sane`.

## Transport

- **Firecracker:** the socket is the VM's vsock path for host port 3128,
  mode 0600 in a 0700 dir; a guest connection to CID 2 port 3128 reaches
  it. No other host port has a listener once the box is ready, so
  Firecracker resets a guest connection to CID 2 on any other port. Exec
  and the unmount request run over the VM's own vsock socket, in the same
  directory. pinfold's guest kernel has no vsock loopback, so only the
  host can connect to a guest listener. The host never half-closes an
  exec connection; stdin's end travels as a frame.
- **Apple `container`:** `--ssh` with `SSH_AUTH_SOCK` set to the proxy socket
  for that one `container run`. It appears at
  `/var/host-services/ssh-auth.sock`, root-owned and not connectable by the
  box user; the supplied init's fixed operation in step 6 changes its mode.

Socket directories are user-owned 0700. Sockets are created 0600. Bind,
validation, vsock connection and VMM mounting use retained directory or socket
descriptors; no final-component symlink is followed. The host socket parent
is root-owned and either sticky or not writable by other users. Unsafe paths
refuse as `runtime-path`. Socket paths must fit the platform address structure,
including suffixes and terminator.

Socket binding and non-atomic close-on-exec setup exclude concurrent
process creation.

Cleanup is authorized by the complete generation in a matching marker or in
the directory name. Published directories are renamed without replacement to
a full-generation retiring name before unlinking. Full-generation staging and
retiring names authorize cleanup with an absent or short marker. Cleanup
checks socket type and UID, regardless of mode. An unverifiable shortened
name is preserved unchanged. Once the runtime is removed and reaped, its
name-locked dead claim is retired. Pruning reports each failure and continues.
Cleanup inventories generation socket directories and sweeps inactive
generation-authorized directories without records under the same checks.
Before publishing a Linux box claim, the owner durably records that no VMM has
started. A VMM child waits on a pipe before entering its jail. The owner
atomically replaces and syncs its identity before releasing that child;
EOF exits without exec. Retirement requires a known pre-start state, a
complete identity proving the VMM ended, or successful owned reaping.
Unknown liveness preserves the generation's resources.
Private socket staging may be removed after a pre-start publication failure
without retiring the claim. Before deleting other generation resources,
confirmed runtime removal under the name lock is recorded by atomically
marking the full-generation owner record retiring and syncing its directory.
Box and builder cleanup resume from that record even after the identity tree
is gone. The owner record and its retirement evidence are unlinked last and
the directory is synced.
Readiness inspects only its own claim. Inventory reports per-box errors and
rechecks retirement before treating a missing identity as unknown. Unknown
liveness retains disk ownership and is reported as `unknown`.
Inventory and measurement report bad records and continue. A resource decision
consults only claims that can reference that resource. Unknown VMM liveness
pins that claim's generation state and named disks. A wholly unreadable claim
prevents deletion or attachment of any existing managed disk because its attachments
are unknown. The attachment refusal names the bad record. Starts using only
new disks, reporting and recovery of other verified-dead claims continue.

## Egress proxy

In the pinfold process, one per box, listening only on the box's socket. No
network listener, no token: the socket identifies the box.

- **CONNECT:** port 443 only, to an allowlisted host (exact name, or
  `.suffix` for the name and its subdomains). The ClientHello SNI must equal
  the CONNECT host.
- **Plain HTTP:** port 80 only, one request per connection, Content-Length
  framing only. Ambiguous framing and bare LF get 400, as does any head the
  parser rejects, folded headers included; a CONNECT head is parsed the
  same way.
- **Refused:** IP literals, and names resolving to loopback, unspecified
  (`0/8`), private, link-local, CGNAT (`100.64/10`), benchmark
  (`198.18/15`), IETF protocol (`192.0.0/24`), multicast, reserved (`240/4`,
  with broadcast), site-local (`fec0::/10`) or NAT64 local-use
  (`64:ff9b:1::/48`) addresses. An IPv6 address that embeds an IPv4 one
  (IPv4-mapped, IPv4-compatible, NAT64 `64:ff9b::/96`, 6to4) is checked as
  that IPv4 address. Resolve once; dial the checked address.
- **Routes:** names are unique ignoring ASCII case; a collision refuses
  `up` as `spec` before creation. A name maps to one host service, e.g.
  `api.internal → 127.0.0.1:7777`. Plain HTTP only, Host header rewritten,
  `Upgrade` dropped (no WebSocket). CONNECT to a route is refused. The host
  service authenticates its callers.
- **Injecting routes:** a route whose value is `{ "to": ORIGIN, "headers":
  { NAME: { "from": VAR, "prefix": "…" } } }`. `to` is an `http://` or
  `https://` origin. The box sends `http://<route>/path` as for any route;
  the proxy dials `to`, over TLS for `https` with SNI and the certificate
  checked against the host's roots, sets `Host` to `to`'s authority, and
  sets each named header to `prefix` plus `$VAR` from `up`'s own
  environment, replacing any the box sent under that name. Values are read
  once at start; a missing variable refuses `up`. An `https` target is
  resolved and checked like an allowlisted host; an `http` target is a host
  service. The value never reaches argv, the box, the spec or the log. The
  box can use the credential but not read it, unless `to` echoes request
  headers back. Header names that frame the request (`Host`,
  `Content-Length`, hop-by-hop) are refused.
- **Login routes:** a route whose value is `{ "login": AGENT }` carries
  a login for that agent; pinfold owns the origin and named authentication
  channels. A spec has at most one login route per agent, or for Pi one per
  Pi provider; otherwise `spec`. An optional `to` replaces the agent's origin,
  checked like an injecting route's target. Routes generate native configuration
  for the whole box. Agent precedence remains in force: Claude's managed env
  overrides user settings and inherited env; Codex's user config, supported
  project settings and explicit options override its system layer. Project-local
  `model_provider` is ignored. Pi's stored credentials override extension placeholders. Configuration is convenience, not credential isolation.

  Every login route applies these rules. Validation precedes credential
  refresh, DNS resolution and an upstream connection:

  - Remove every occurrence of `Authorization`, `X-Api-Key`, `Api-Key`,
    `X-Goog-Api-Key`, `X-Auth-Token`, `X-Access-Token`,
    `ChatGPT-Account-ID`, `OpenAI-Organization`, `OpenAI-Project`, `Cookie`
    and `Cookie2`, and every header whose name begins `Proxy-`, ignoring
    ASCII case. Drop whole fields, including comma-separated values.
  - Collect every `Connection` field before dropping hop-by-hop headers.
    Split its values on commas, trim SP/HTAB, ignore empty members and
    validate every remaining member as an HTTP token. Malformed options
    return 400, reason `malformed request`. A nomination of `Content-Length`
    returns 400, reason `ambiguous framing`. Remove other nominated guest
    fields, ignoring ASCII case. Regenerate `Host` and append trusted
    authentication after filtering; a nomination cannot suppress it.
  - Split the raw query on `&` and `;`, then each field at its first `=`,
    including fields without `=`. Strictly percent-decode names once,
    map `+` to space, compare ignoring ASCII case and treating `_` as `-`.
    Refuse these names with 403, reason `login credential query`: `key`,
    `api_key`, `api-key`, `apikey`, `x-api-key`, `access_token`,
    `refresh_token`, `id_token`, `auth_token`, `bearer_token`, `token`,
    `client_secret`, `authorization`, `chatgpt-account-id`,
    `openai-organization` and `openai-project`. Validate percent escapes
    throughout the query; any invalid escape returns 400, reason
    `malformed request`. Preserve accepted query bytes exactly. Do not
    recursively decode, inspect bodies or restrict paths.

  Only the trusted authentication fields below are appended. Guest and
  host credentials are never merged. Refusal reasons contain no query
  text, header values or helper output.

  - `claude`: `{ "login": "claude", "from": VAR }`, where `$VAR` is a
    `claude setup-token` token read once at start like an injecting route's.
    The route goes to `https://api.anthropic.com` with `Authorization:
    Bearer`. Claude's managed settings set `ANTHROPIC_BASE_URL=http://<route>`,
    a placeholder `CLAUDE_CODE_OAUTH_TOKEN`, and blank `ANTHROPIC_API_KEY`.
  - `codex`: `{ "login": "codex" }` uses the host's own Codex login, in
    whatever store Codex keeps it. The proxy gets the access token from
    the pinned host helper (Pinned artifacts): `codex-app-server` with the
    built-in `openai` provider forced and `up`'s environment, sent
    `initialize` and `getAuthStatus { includeToken: true }` on stdio. Codex
    refreshes and writes its own store; pinfold never reads it, and holds
    the token in memory only. The token's JWT must carry a future `exp`
    and an account id; the route goes to `https://chatgpt.com` with
    `Authorization: Bearer` and `ChatGPT-Account-ID`. Within 5 minutes of
    `exp`, the proxy asks the helper again under one host-wide lock, at
    most once a minute per box; a request on a lapsed token is refused and
    logged `login lapsed`. An ask that encounters the held login lock prints
    `login-busy` once on stderr before waiting. Lock acquisition and helper
    exchange share a 30-second deadline. Cancellation kills and reaps the helper. Helper
    stdout is bounded to 256 KiB total and 64 KiB per line; helper errors
    never include its output. The box gets `/etc/codex/config.toml`, mounted
    read-only, selecting a custom provider with `base_url =
    "http://<route>/backend-api/codex"`, `wire_api = "responses"` and
    `requires_openai_auth = false`; it is Codex's system layer, so the
    caller's own config still overrides it.
  - `pi`: `{ "login": "pi", "provider": P, "from": VAR }`, where P is
    `openrouter`, `anthropic`, `openai` or `anthropic-subscription` and
    `$VAR` is that provider's API key, or for `anthropic-subscription` a
    `claude setup-token` token, read once at start like an injecting
    route's; or `{ "login": "pi", "provider": "openai-codex" }`, which
    uses the host's Codex login as a `codex` login route does: helper,
    in-memory token, JWT checks, refresh window, host-wide lock,
    `login-busy`, `login lapsed`, deadline and reaping. With a `codex`
    route in the same box, both share one token and one refresh: the
    helper is asked once at start. A missing or other
    P, `from` with `openai-codex`, or two routes for one pi provider
    (`anthropic` and `anthropic-subscription` are both pi's `anthropic`)
    is refused as `spec`, and so is `provider` on a claude or codex login
    route.

    | P | pi provider | Origin | Header set |
    |---|---|---|---|
    | `openrouter` | `openrouter` | `https://openrouter.ai` | `Authorization: Bearer $VAR` |
    | `openai` | `openai` | `https://api.openai.com` | `Authorization: Bearer $VAR` |
    | `anthropic` | `anthropic` | `https://api.anthropic.com` | `X-Api-Key: $VAR` |
    | `anthropic-subscription` | `anthropic` | `https://api.anthropic.com` | `Authorization: Bearer $VAR` |
    | `openai-codex` | `openai-codex` | `https://chatgpt.com` | `Authorization: Bearer` and `ChatGPT-Account-ID` from the host login |

    Remaining headers, including `anthropic-version` and `anthropic-beta`
    unless nominated by `Connection`, pass unchanged. Every path under
    the origin is forwarded, responses stream as received, and `Upgrade`
    is dropped. These rules cover only the named authentication channels,
    not arbitrary upstream authentication conventions.
    Pi's autoload extension sets every route's provider `baseUrl` to
    `http://<route>` plus its built-in base path (`/api/v1` for openrouter,
    `/v1` for openai, none for anthropic, `/backend-api` for openai-codex).
    Each provider receives a placeholder key; openai-codex's is an unsigned
    JWT with a placeholder account id. The extension holds no secret and
    needs no `-e` argument. A stored `/login` credential still wins in Pi;
    the route replaces its token, and Pi's refresh bypasses the route.
    Generated file contents never enter the home disk. Seeds, reseeding and
    `/login` are unchanged.
  - `up` refuses a login it cannot use (no token, an empty or unset
    `from` variable, or not a usable JWT) as `login`, naming the agent.
- **Limits:** a connection cap and a header timeout. A tunnel is idle, and
  closed, only when neither direction has carried bytes for 5 minutes.
  A write blocked for 5 minutes closes the connection even when the other
  direction has activity. A fatal relay error closes both directions.
- **Log:** one JSON line per decision in the run's egress log at
  `~/.local/state/pinfold/runs/<run_id>/egress.jsonl`, so a reused name
  starts an empty log. It names the decision's UTC
  time (RFC 3339 to the second), the host, the decision and its reason. A
  route request's line also names the method, the request path without its
  query, and the upstream's status (null when no status line arrived). No header value is ever written.
  Each box's log is capped at 128 MiB. A complete encoded JSON line,
  including its newline, that would exceed the cap writes nothing. A line
  that fits exactly is appended. There is no rotation or truncation.
  An append failure or cap exhaustion stops the box with `audit-log`.
  Either can prevent recording a request already forwarded to its upstream;
  teardown follows the owner notification.

## Images

An image is a toolchain and may package agents. A project selects an explicit
`image`, otherwise the project's Containerfile, otherwise the nearest
Containerfile in the profile's chain (Profiles). An explicit image and project Containerfile together refuse
as `image-source-conflict` after layering. An explicit image overrides the
profile recipe. No source refuses as `image-source-missing`; an empty image
refuses as `image-invalid`, before state is created.
Explicit images resolve locally. A project launch never pulls or imports an
image itself; a confirmed managed build can pull its recipe's external base.
A project recipe typically `FROM`s the profile image.

- When starting a new box with interactive I/O (Configuration),
  `pinfold -- CMD` offers to build a missing managed image. Confirmation
  runs the selected recipe once and retries admission with the original
  argv. Declining exits 125. A failed build exits without running CMD. A missing explicit image names
  the image and requires local preparation. Without interactive I/O a missing
  managed image refuses and names the build command.
  `pinfold build` builds the selected recipe. An explicit image refuses as
  `image-no-recipe`.
  `pinfold build --profile NAME` explicitly builds that profile's chain
  (Profiles), or refuses as `image-no-recipe` when no profile in it has a
  Containerfile.
- A profile or project build gets an empty context: the Containerfile
  alone. Files come in by `ADD --checksum` or from the profile image.
  Every managed build re-resolves built-in agent versions through its profile
  recipe. An isolated resolver stage
  writes only versions and their integrity metadata; installation uses that
  file and reruns when its resolved versions change or with `--no-cache`.
  Other recipe steps use the layer cache. Managed `build --no-cache` forces
  the full recipe to run. Launch never installs agents. A managed build
  first rebuilds, root first, every profile beneath it that has a recipe:
  a project build its selected profile's chain, a profile build its base
  chain. Each image records the identity of the one built just beneath it,
  and Linux freezes that identity for its build.
  Each image's publication is a separate operation: a failed build may leave
  successfully rebuilt images beneath it published, reported on stderr.
- After each completed Apple build, pinfold attempts to return unused
  builder blocks without pruning cached layers; failure only warns and
  preserves the build result.
- A build's identity is its unique tag, never an image label. Every
  build is tagged `<repo>:<build>` and moves the stable `<repo>:latest` to
  it. A caller build whose layers and labels are unchanged returns the
  existing image.
- Every build sets `dev.pinfold.profile`, `dev.pinfold.project` and
  `dev.pinfold.base`; a label not its own is empty:

  | Build | Repo | `profile` | `project` | `base` |
  |---|---|---|---|---|
  | profile | `localhost/pinfold/profile-<name>` | `<name>` | | image id of the nearest base with a recipe, when there is one |
  | project | `localhost/pinfold/project-<id>`, the project's state id | | `<id>` | selected profile's image id, when its chain has a recipe |
  | caller | `localhost/pinfold/image-<NAME>` | | | |

- A profile build's stable ref is what a project Containerfile `FROM`s
  and what `doctor` compares against.
- A caller image is built by `pinfold image build NAME` from a context
  directory the caller vouches for, as it vouches for its mounts: no
  project and no project configuration. The Containerfile may lie outside
  the context. NAME is validated like a profile name. A box spec's `image` names
  either tag.
  Identical inputs under two names build one image. It carries the
  caller's `--label`s, none of which may start with `dev.pinfold.`. It uses
  the builder's layer cache unless the caller passes `--no-cache`.
- Before invoking the builder, a build names its profile, project or caller
  image on stderr. A build never touches project homes.
- Profile and project images record their Containerfile's SHA-256 as
  `dev.pinfold.containerfile-sha256`. Before a project-command launch, and in doctor,
  changed or unrecorded Containerfile inputs produce an `image-outdated`
  hint with the rebuild command. A launch checks every image beneath the
  selected one the same way, even before it is rebuilt. An image whose
  recorded base is not the current base image's id, that records a base
  when its chain now has none, or whose base is itself outdated, also
  names `image-outdated`, so a change at a chain's
  root names every image above it. Explicit
  images get no recipe freshness hints. Hints never refuse the launch.
  Harness pins and home settings are not image inputs.
- On Apple, concurrent pinfold builds do not interrupt each other when
  callers set different `NO_COLOR` or `BUILDKIT_COLORS`.
- Apple base refresh pulls only the host platform unless the first `FROM`
  names an explicit platform. An unresolved platform is not pulled.
  The empty `scratch` base has no digest and is not pulled.

### Linux image preparation

This is a separate internal role, never a box-spec option. Agent Always
applied and outer VMM jail, parent lifetime, seccomp and vsock controls
stay exact. The trusted guest service stays root with capability bound
`0xa82425fb` (mount included) and `no_new_privs`. RUN uses pinned
BuildKit's ordinary OCI capabilities without SYS_ADMIN; user namespaces
and insecure entitlements are disabled. There is no NIC or direct route.

Attach only the pinned immutable bootstrap, this binary's read-only init,
host-written read-only inputs and owned writable storage. No host bind,
agent home, host auth, registry credential, agent disk or unrelated device
enters either role. A builder exclusively owns one version-keyed cache;
a converter gets a fresh separate work disk and never attaches that cache.
Ownership, cancellation, audit failure and recovery remain host authority.
A job must stop and reap its VMM before releasing ownership. Discard the
derivable cache after unclean work; completed images survive cache removal.
A failed cache job prunes unused mutable cache mounts through pinned BuildKit,
preserving regular image-layer cache. Cleanup failure omits the clean end receipt;
the next build recreates the cache. A handled recipe failure may retain the cache
only after daemon/work teardown, sync and unmount, a complete failure/end/EOF
receipt, VMM reaping and a successful host backing-file sync. A stop during
clean-marker updates attempts to restore the marker, then retire the backing file
if restoration fails; errors propagate.
A stop after those physical barriers does not make the cache unclean again.

Daemon and RUN use the production proxy and audit. Initial build hosts are
`registry-1.docker.io`, `auth.docker.io`, `production.cloudfront.docker.com`,
`docker-images-prod.s3.dualstack.us-east-1.amazonaws.com`,
`deb.debian.org`, `github.com`, `api.github.com`,
`release-assets.githubusercontent.com`, `downloads.claude.ai`,
`registry.npmjs.org` and `cli.github.com`. Linux `build` and `image build`
accept repeatable `--allow HOST` exact DNS additions, independently of agent
policy. A recipe grants no network authority. Denials retain the proxy's
stable reasons. Apple rejects `--allow` additions and retains its existing
build egress policy. Build records name their audit log on stderr and use
normal retained run records. Each Linux build's egress log has the same
128 MiB cap. Exhaustion follows the existing audit-failure handling and
publication boundary below. No credential values enter argv or files.

Freeze caller context and local image metadata. One finite builder guest
uses pinned BuildKit's native Dockerfile frontend for discovery and build.
Initially it receives local aliases and opaque identities, without local
image configurations or layers. Its helper requests exact frozen configs
on demand; the host records each consulted local identity before replying.
Every consultation counts, including empty-rootfs and other optimized-out
inputs. Later native metadata calls may only reuse an already served local
binding. Metadata lookup closes permanently before RUN.

Lease and validate those identities, then supply their complete OCI contexts,
including normalized `:latest` names. Deleted identities fail preparation;
they are never retargeted. The selected job and source policy are host-written
on a separate physical read-only device attached blank at boot. The host
formats private staging, copies its bounded bytes through the retained backing
FD without resizing or replacing it, syncs and closes writers before authorizing
use. The backing reservation derives from the existing input and filesystem
overhead bounds. Init leaves that device unopened until authorization, then
invalidates its quiescent block cache and mounts it read-only. An acknowledgement
does not establish host completion. A new native build session uses only the
selected OCI contexts. A source policy refuses managed-image registry fallback
and pins discovered external image digests for the build pass.
Linux uses the pinned native frontend; `#syntax` cannot select another one.
The base profile dependency is frozen to the identity recorded in
`dev.pinfold.base`; a changed or missing dependency is `image-base-changed`.
Apple records the label but does not bind the build to it; its native
builder resolves the profile tag.
On both platforms that public label is empty for caller builds and for a
profile build with no recipe beneath it; only managed builds record a
local profile identity.
Other resolved image digests are internal metadata, never user configuration.
Linux records the reachable image inputs from native BuildKit discovery,
then freezes those identities for execution. This is not an attestation or
an independent provenance claim.

Guest output is a bounded framed stream. The host creates only fixed-mode
staging paths derived from validated kinds and SHA-256 values, ignoring
all guest path, owner and link metadata. It never extracts a guest archive
on the host or reads/mounts a guest-writable disk. Validate sizes, hashes,
media, linux/amd64 config, expected labels and the complete local descriptor
graph. No external descriptor URL or unreferenced object is admitted.
Admit one direct runnable manifest. Attachments, attestation manifests,
nested indexes and unsupported media are refused rather than ignored.

The converter has a 7 GiB work disk. A completed raw root is at most
2 GiB. An admitted image's aggregate OCI payload is at most 1 GiB;
selected local-base payloads together are at most 1 GiB. Caller context
retains its separate 512 MiB bound. Snapshot accounting includes filesystem
allocation and directory overhead.

The fresh independent converter receives only the host's validated read-only OCI layout,
has no egress, and uses pinned BuildKit flattening plus guest tar and
mkfs.ext4. Conversion uses the admitted layers without executing inherited
image triggers; the original OCI configuration stays unchanged. Ownership,
modes, links, whiteouts, capabilities and ACL metadata must survive conversion.
Receive the raw root as opaque bounded hashed
bytes. Reuse only a completed representation with matching manifest,
converter recipe/bootstrap and verified raw-root size/hash.

Publish immutable OCI objects and completed root before one atomic catalog
generation containing the unique ref and latest together. The catalog
rename commits the build. Before commit, failure, truncation, deadline or
interruption changes neither ref. After commit, late stop, audit, durability
or cleanup errors do not change its successful outcome; durability and
cleanup problems are reported. Running-image and
build-input leases prevent removal through concurrent clean/rm/publication.
Starting a prepared image reads only the catalog and completed root/config
and works with the builder absent. Damaged preparation requires a rebuild.
Partial roots are never reusable.

The cache is `$XDG_CACHE_HOME/pinfold/images/`: `catalog.json` maps refs to
immutable manifests and records labels, config environment, resolved image
inputs and completed root derivation. There is no registry or resident
daemon. Old Podman-only refs no longer resolve: rebuild profiles/projects
with `build` and caller images with `image build`. The old Podman store,
old derived-root cache and managed agent disks are left untouched.

Built-in images use `debian:trixie-slim` and strip setuid/setgid bits.

| Profile | Tools |
|---|---|
| `default` | shell, ca-certificates, git, curl, ripgrep, fd-find as `fd`, jq, less, procps, standalone Pi, native Claude, Codex and its code-mode host |
| `documents` | default tools plus Bun, AnyDoc and Poppler |
| `adam` | documents tools plus gh, fff (pi extension) and ponytail |

`profile/image/<name>.Containerfile` is each built-in's recipe. `documents`
has `base = "default"` and `adam` has `base = "documents"`; each recipe
`FROM`s its base's stable ref and adds only its own tools.
Every built-in recipe ends with the step that strips setuid/setgid bits,
so each link strips what it added. A user recipe's steps are its own.
Infrastructure and tool assets use pinned `ADD --checksum` inputs for the
selected architecture. Agents resolve stable release metadata on every managed
build. Only `default`'s recipe resolves and installs agents; the profiles
above it inherit them from its image. Since every managed build rebuilds
its chain from the root, a build of any profile on `default` reruns the
resolver. A per-build argument is declared only by the resolver stage; no other
step depends on it. Installation is cached by the resolved version file and
uses publisher integrity metadata where supplied. Codex and its code-mode
host come from the same resolved release. A recipe not built on `default`
controls its own agent installation and must retain the resolver stage for
this behavior.
Missing required assets fail installation.
Final images contain installed tools, without their archives. gh comes
from GitHub's signed apt repository.

## Pinned artifacts

Image agents resolve at build time and report their own versions. Only the
host Codex authentication helper remains an agent-related pin. It never
enters a box.

Artifact and update downloads share a bounded HTTPS-only downloader.
It ignores curl configuration, rejects HTTP redirects, caps artifact
transfers at 512 MiB and 5 minutes, and checks SHA-256 before installation.
Cancellation kills and reaps the transfer before startup cleanup.

`crates/pinfold/host-helper.toml`, embedded at build time, pins the Codex
host helper's version and host-architecture assets. The verified installer
keeps it in `~/.cache/pinfold/artifacts/codex/<version>/<host-os-arch>/`.
A missing recorded file reinstalls it. A checksum mismatch fails the login.

`scripts/bump-pins.py` moves the host helper, infrastructure, image-tool,
Rust, Cargo and Public Suffix List pins only with a pinfold release. Versions
only move up to stable releases and are never rehashed at the same version.
The helper takes the latest stable release once its assets exist; other image
tools wait seven full days after their last required asset was published.
A missing publication time makes them ineligible. Cargo dependencies take
the latest stable, non-yanked version. Publisher integrity failure aborts
before any pin file is written.

On Linux, `crates/pinfold/vmm.toml` pins Firecracker and its guest kernel
for `linux-x64`, each checked against its pinned digest. The guest kernel
is pinfold's own build of a pinned kernel.org stable release, verified
against kernel.org's signature and published as a prerelease, never the
latest release, without
vsock loopback and with its config at `/proc/config.gz`; `guest-kernel/`
builds it. Both use the verified installer, into
`~/.cache/pinfold/artifacts/vmm/<version>/<os-arch>/`, where `<version>`
names Firecracker's and the kernel's, on the first `up`.

`builder-root/` separately builds a Linux x64 bootstrap from a signed Debian
snapshot, an exact package closure and pinned BuildKit binaries. Pinned Go
compiles its metadata helper offline against an exact BuildKit source/vendor
tree, with local toolchain selection and module downloads disabled. Go and
that tree are producer dependencies for resolving configuration-only bases.
The source archive has a measured SHA-256 pin and the publisher's verified
signed-tag/commit record; Go has its publisher's checksum. The helper source
joins the recipe identity. The tar.gz contains only the 512 MiB read-only ext4
root; the manifest and SHA256SUMS cover build metadata and retained producer inputs.
The workflow only builds. It requires Linux root, Python 3.11.4+ with
`tarfile.data_filter`, debootstrap, GnuPG, curl 8.4+, e2fsprogs and GNU tar.
The consumer needs no host Go installation.
`crates/pinfold/builder.toml` embeds the published archive URL and reviewed
archive, raw-root and recipe identities. Pinfold acquires the archive over HTTPS
into install staging. The offline archive cache
is `~/.cache/pinfold/artifacts/builder/<archive-sha256>/`; when present, its
archive is copied into that staging area instead. The pinned archive bound
is 122,074,383 bytes, below the shared 512 MiB transfer cap. Acquisition,
verification and installation share one cancellable 300-second deadline.
Installation verifies the staged archive's pinned size and SHA-256 before
streaming only `root.ext4` into a fixed 0600 file and atomically publishing
the complete `linux-x64/` install. The raw-root size and SHA-256 are verified
before publication and on every use; a mismatch fails with
`builder-integrity`. Embedded digests remain authority if public bytes
change; the sidecar manifest supplies none. `doctor --artifacts --json` reports
the builder URL, archive pin/size, recipe SHA-256 and verified cache state.

codex also pins a host helper, `codex-app-server`, per host `os-arch`
(`darwin-arm64`, `linux-x64`), installed the same way into
`~/.cache/pinfold/artifacts/codex/<version>/host-<os-arch>/` on the first
`up` with a codex login route or a pi `openai-codex` login route. It
never enters a box.

The init is not an artifact: it always comes from the CLI's own build
(Code). The macOS CLI writes its embedded init to the cache once; a Linux
CLI writes its own executable into a read-only init disk.

## Profiles

A profile works the same for interactive runs and programmatic boxes,
with or without a TTY.

```
~/.config/pinfold/profiles/<name>/
  pinfold.toml    config defaults (Configuration)
  Containerfile   optional image recipe
  home/           seeds for $HOME
  share/          mounted read-only at /opt/pinfold/profile, as of each start
```

- `default`, `documents` and `adam` are built into the binary from
  `profile/`. A user profile with the same name replaces the whole
  built-in, also as another profile's base.
- `base = "NAME"` in `pinfold.toml` names the profile this one builds on,
  built-in or user. A base may have its own base. A cycle, a self-reference
  or a missing base refuses as `profile`. The chain applies root first:
  each profile's `pinfold.toml` keys replace its base's as the project's
  replace the profile's (Configuration); its `home/` and `share/` files
  lie over its base's by path, whole files; an upper path also replaces
  every lower entry beneath it or above it on that path, so a file and a
  directory never meet. A share from one profile in the
  chain mounts as that profile's own; shares from two or more mount as a
  copy of the merged files, bytes and executable bit, written at each
  start into the box's state dir and removed with it. A
  profile's Containerfile `FROM`s the stable ref of its base's image;
  pinfold does not read the `FROM`. A profile with a Containerfile whose
  direct base has none refuses as `profile`. A profile without one uses
  its nearest base's image. The box spec's `profile` applies the whole chain.
- `pinfold profile new` writes a copy of `default`, or of the profile named
  by `--from`: that profile's own files and `base`, not its base's files.
- A profile whose chain has no recipe needs an explicit image or, for project commands, a project recipe.
  Without either, core refuses as `profile`, with `image-source-missing`.
- `home/` needs `$HOME` on a writable disk; seeds (Terms) are copied in at
  each start. Changing a profile does not reset disks or overwrite saved seeds.
- pinfold does not manage pi settings: `pi install` in a box changes that
  project home only.
- Built-ins package all three agents on PATH. Missing-only home seeds
  supply Pi's trust/settings defaults, Claude permission/sandbox defaults
  and Codex approval/sandbox/update-check defaults. Native home instructions
  are `.pi/agent/APPEND_SYSTEM.md`, `.claude/CLAUDE.md` and `.codex/AGENTS.md`.
  An existing instruction file blocks its missing-only seed, including an
  earlier seed; changing profiles does not update it. Built-in instructions
  describe both mount kinds. Share edits already affect host files, and box
  commits land on the host's branch. Disk commits reach the host through
  `git fetch box`. Instructions are guidance and do not enforce either boundary.
  Supported updater switches are recipe environment defaults. Route-generated
  system files contain authentication configuration only.

pi's two config levels both load in every run: the user level (pi's agent
dir under `$HOME`, seeded from `home/`, with packages from `share/`) and
the project's own `.pi/` and `.agents/skills`.

Every built-in seeds only `/opt/pinfold/profile/pi` as its pi package and
sets `defaultProjectTrust: "always"`. It adds
pi's built-in `codemode` tool to the default tools; pi's built-in MCP is
on by default: stdio servers run in the box, remote ones through the
proxy. Every built-in includes the operating-context extension, which
writes the box's facts into the system prompt (the allowlist from
`PINFOLD_ALLOW`, that a 403 is final, and the share/disk guidance above).
The extension retains project guidance when an existing native instruction
file blocks its missing-only seed. Only `default` carries home seeds;
the others inherit them. Documents adds `read-documents`, which adam
inherits. Adam's own `pi/package.json` replaces default's and also
registers fff and ponytail.

Default and documents set no config besides `base`. Adam's `allow` is the default list
plus `github.com` and `api.github.com`. `PI_OFFLINE` is unset, so pi's
package installs go through the proxy.

## Disks

On Linux a disk is an ext4 image at
`~/.local/state/pinfold/disks/<name>.ext4`, mode 0600, created by
pinfold; on macOS it is an Apple `container` volume named
`<first 16 hex digits of the sha256 of the canonical state dir>-<name>`. The next `up` naming a disk pinfold created whose box never came ready treats
it as absent: it removes it and creates it again. Before `ready` the guest
syncs each new disk's copy. On Linux pinfold then fsyncs the disk's image,
so a disk whose box came ready holds its whole `from` after a host power
loss; on macOS what a volume keeps through one is the runtime's. A disk has no
labels and no owner beyond its attachments. A disk, its file or volume, its attachments and
its records belong to one state root; another root naming the same disk
gets its own.

- **Attachment:** at `up`, after the claim. One box may attach a disk writable; while it does,
  another `up` naming that disk is refused as `disk-in-use`. Boxes may
  share a disk read-only while none attaches it writable. Attachments are
  recorded under the box's claim and count until confirmed runtime removal
  and published retirement, or removal of that claim. A readable claim pins
  only its named disks, regardless of VMM liveness. A wholly unreadable claim
  refuses deletion or attachment of any existing managed disk. Attachment refuses as
  `disk-in-use`, naming the bad record. Starts using only new disks and
  measurement continue.
  An Apple preparation error removes new disks and claimed state when no helper
  container was created or its removal is confirmed. If stop/removal cannot
  confirm an existing helper is gone, the claim and disks remain for recovery.
  Preparation stderr retains only its latest 64 KiB in memory and
  is escaped when reported.
- **Clean:** a writable attachment records the disk unclean. A stop
  pinfold requests of a running box (step 8) syncs and unmounts every
  disk, and pinfold records each clean once the runtime confirms it. A
  disk whose writable box ended on its own, or whose owner died, stays
  unclean: a read-only attachment is refused as `disk-unclean` until a
  writable one, which recovers the disk, ends with `down`.

### Crossing

A disk's `from` copies host files in once, when pinfold creates the
disk; the pi layer's disks name none. Otherwise data crosses only as git
branches, through a disk project's Git remote (Git), against a live box
and run in it as the box user. Neither direction touches a working tree.

- **Push** bundles the host refs Git names and fetches them in the box
  into the box repository's `refs/remotes/host/*` only, in one
  transaction, with no tags. The box merges or rebases them itself. The
  host reads nothing from the box but its exit status, and prints none of
  the box's text.
- **Fetch** has the box bundle its repository's branches, less what the
  host's `refs/remotes/NAME/*` already hold, where NAME is the remote.
  The box first lists its branches: lines of at most 4096 bytes, each
  `OID refs/heads/BRANCH` with a full object id of the host repository's
  width (40 hex digits for SHA-1, 64 for SHA-256), then an empty line. More
  than 4096 listed branches, or one listed twice, fail the fetch. Every
  listed name must pass `git check-ref-format` under `refs/remotes/NAME/`
  before git sees it. The bundle's heads are branches the listing names,
  each at its listed commit; any other head fails. Tags, symrefs and
  other refs stay in the box. A bundle past 4 GiB is refused as `limit`, and the
  box's command is killed. On the host, pinfold runs `git bundle verify`;
  a filtered bundle fails. It then runs `git index-pack --fsck-objects`
  of the bundle's pack and `git fetch` of the bundle, both into a
  quarantine object directory inside the repository's, so fetch's
  connectivity check runs there too. That fetch names no destination and
  runs with `--no-prune --no-prune-tags --no-tags`, so it writes and
  deletes no ref whatever the user's configuration. Every listed branch
  names a commit, never another object type: a bundle head the box's
  pack carries itself (not a base `--fix-thin` added from the host),
  which the quarantined fetch checked connected, or one
  `refs/remotes/NAME/*` already reaches. An object the host holds
  otherwise is never the box's to name. A bundle carries
  no hook or config, and nothing is checked out. Only branches that pass
  every check reach Git, and the quarantine's packs move into the
  repository only when Git asks for their objects: each index after its
  pack, a partial move undone, and a quarantine another fetch swept away
  fails. Moves into one object store run one at a time, under a lock on
  its `pack` directory. A rejected bundle leaves the repository's objects and refs as
  they were. pinfold removes the quarantine as the helper exits; the next
  fetch removes one whose process is gone. Fetch has no deadline or
  memory bound: a box can stall it, or load the host, with a bundle slow
  to write or costly to index, until the caller kills it; a kill before
  Git writes changes no ref. The repository's own hooks and config are
  the user's and apply as to any fetch.

`disk` prints human-readable output by default. `list` shows each disk's
allocated storage, capacity, clean state, attachments and recorded full
origin path. A project origin is its canonical root. Caller-created disks
may have no recorded origin. Paths are descriptive,
never deletion authority, and terminal controls are escaped. Human sizes
use B, KiB, MiB and GiB as appropriate.

A trailing `--json` selects structured output. `disk list --json` prints
one line per disk, with exact byte counts:

```json
{"name":NAME,"size":BYTES,"allocated":BYTES,"attached":[{"box":NAME,"readonly":BOOL}],"clean":BOOL}
```

`disk rm NAME` removes a disk no box attaches, and exits 0 for an absent
one; an attached disk is refused as `disk-in-use`, exit 1. `rm --all`
snapshots the managed disks in this state root and attempts each ordinary
removal. Attached disks stay intact; failures do not stop other removals,
and any failure makes the command exit 1. Disks created after the snapshot
are outside that pass. Removal is not atomic across disks.
With `--json`, removal prints:

```json
{"event":"removed","disk":NAME,"existed":BOOL}
{"event":"refused","disk":NAME,"reason":"disk-in-use"|"spec","detail":TEXT}
```

`disk export NAME DEST [--image IMAGE]` copies a stopped disk's files into
a new host directory. The user explicitly requests these untrusted bytes;
this is the exception to the no-secret-file rule for exported file contents,
not for argv, diagnostics or forwarded credentials. It uses only IMAGE
(default `localhost/pinfold/profile-default:latest`), no profile,
egress, host environment or host destination mount. A missing image is not
built or pulled automatically. The supplied init sends files from a read-only
attachment. Existing, filled, clean and unattached are checked under the
disk claim lock; missing/unfilled, attached and unclean disks refuse as
`disk-missing`, `disk-in-use` and `disk-unclean` respectively.

The host validates at most 65536 entries, 4096 bytes per path or symlink
target, 16 MiB of path/target metadata and 64 GiB of file contents in total
and per file. The transfer has a 300-second deadline after box startup.
Invalid framing, truncation, duplicate paths, ancestor or host-name collisions,
special files and traversal fail. Symlinks are copied as links, never followed.
An inaccessible, root-owned top-level `lost+found` is formatter housekeeping:
export preserves that directory as empty, not its recovery contents. A
user-owned namesake is exported normally.
Files become 644 or 755 according to their executable bit, directories 755;
ownership and special metadata do not cross. Private staging is removed on
catchable failure; a hard kill may leave private staging, never a partial
DEST. After successful transfer and teardown, publication must not replace
any existing DEST. The destination root is 700. Export does not repair an
unclean disk or change Pi's branch crossing.

Success exits 0. With `--json` the line is:
`{"event":"exported","disk":NAME,"destination":DEST}`.

## Updates

`pinfold update --check` reports whether a newer stable GitHub release is
available. `pinfold update` downloads its host binary and `SHA256SUMS`,
verifies the binary, then atomically replaces the resolved executable.
Command symlinks stay intact. Failed verification or installation leaves
the installed binary intact. Releases are from `adamaltmejd/pinfold` over
HTTPS. Neither command needs a runtime or runs maintenance. No downgrade,
automatic installation or privilege escalation. An executable under
Cargo's `target/.../debug` or `target/.../release` is not replaced.

Working host commands hold a shared lock on their executable for their
lifetime. Installation requires an exclusive lock and refuses as
`update-busy` while another working command is active, including across
XDG state roots. A process whose
executable was replaced before it acquired the lock must be rerun.
Checksum failures name `checksum-mismatch`; development builds name
`development-build`.

Before interactive `pinfold -- CMD` launches, with stdin, stdout and stderr all
terminals, pinfold checks at most once per 24 hours and prints an update
notice on stderr. The request has a one-second total timeout. Failed
checks are silent and count toward the interval. Top-level help and version,
development builds and noninteractive commands do not check.
Each such launch also compares all bundled profiles with their last
observed fingerprint. Changes print `bundled-profiles-changed` once,
independently of the network interval. No history means a silent baseline;
self-update records a missing baseline before replacement. Saved project
settings and user profiles are preserved. `PINFOLD_NO_UPDATE_CHECK`
turns off both the update check and the `bundled-profiles-changed` notice. The timestamp and fingerprint live under
`~/.cache/pinfold/update-check`.

## Maintenance

pinfold only removes what it created: its images and boxes carry `dev.pinfold.*` labels,
and its state lives under its own dirs. Each project's state records its
checkout path and last run.

Automatic, never prompting:
- After a build: keep the newest two builds per source (a profile, a
  project or a caller image name), ordered by
  their tags. Remove older builds' tags; an image goes with its last tag,
  and its dangling layers with it. An image a box still uses keeps its
  last tag, and pins only itself; which boxes use an image is read after
  dead boxes are pruned, from every box the runtime lists. A caller build
  in the last hour stays, so the ref its `built` line named still comes
  up; past the hour the newest two rule applies.
- At most once a day, at the start of a working command (except the deferred
  cases below, and not `--version`, `--help`, `doctor`, `config`, `update`, `box list`, `box down`, `box inspect`, `box events`, `--`, `stop`, the private owner entrypoint or `init`): prune boxes whose owner
  is gone (nothing owns its generation's lifetime lock), leftover sockets,
  artifact `<name>/<version>` directories no pin names (`init/` aside),
  builds beyond each source's newest two (the after-build rule, applied
  even when no build follows), and inactive runs' records (below).
  `box up` runs this pass after admission, before startup preparation; an
  admission refusal runs no pass. Project commands defer it until their
  session lease has ended.
  `start` runs it after keep is committed, without a client lease.
  The Git remote helper defers it until the running owner’s mount decision,
  or configuration mount admission when no owner runs. A share refusal runs
  no pass. Signal handlers are installed before this deferred maintenance.

A run is inactive when its generation owns no lifetime lock. An inactive
run's directory goes when its newest file is older than 14 days; then,
while inactive runs together exceed 128 MiB, the oldest goes first, so
the quota can remove an egress log before 14 days. A live run's records
are never removed or truncated. Its egress log has the Egress proxy's size limit.

`pinfold image rm NAME` retires a caller image name: it removes every tag
of NAME, whatever its age, and no other name's.

`pinfold clean` lists sizes, then removes:
- everything automatic, now
- the unused build cache (Apple: prune BuildKit records, keeping the
  shared builder and active builds; Linux: remove only an exclusively
  claimed idle owned builder cache)
- with `--unused AGE`, state and disks of projects not run for that long,
  except a project's with a live box. Never automatic: a project disk
  holds unfetched work, and a home disk sessions and logins.

A project's configuration is the user's; nothing removes it. A project
whose checkout is gone or moved keeps its state and disks;
`clean` lists them, and they go only with `--unused AGE` or `disk rm`.
Other disks go only by `disk rm`; a project home's caches are inside its
disk and not cleaned.

`--dry-run` only lists. Run records are a category of their own. `doctor` shows disk use per category and suggests
`clean --dry-run` above 20 GiB. `doctor` runs no maintenance and changes no pinfold
state or artifacts. Linux reports allocated backing-file bytes without
booting or reading a cache filesystem; catalog inspection needs no runtime.
When runtime checks fail, it skips runtime-dependent image and disk
probes. On Linux, failed preflight also reports missing host requirements
independently of each other: `root` names a run as uid 0, `kvm` an
inaccessible `/dev/kvm`, `mkfs` a missing `mkfs.ext4`, `userns`
unprivileged user namespaces turned off, `userns-apparmor` them restricted
by AppArmor without pinfold's profile, `landlock` no Landlock or an ABI
below 5.

A launch holds shared project ownership before recording its run, through
box teardown. Deleting project state or caches requires exclusive project
ownership and fresh eligibility checks while holding it. A cleanup
measurement does not authorize a later deletion.

The detached owner holds shared project-maintenance and executable ownership
through teardown, including while kept without clients. Daily maintenance for
a project command runs only after its session lease has ended. Cleanup skips its
project; executable replacement remains `update-busy`.

## pi layer

- **Command:** `pinfold -- CMD` runs the complete literal argv in the image.
  The executable word must be nonempty. No shell or implicit agent is added.
- **State:** a project has host state under
  `~/.local/state/pinfold/projects/<name>-<hash>/` and a persistent
  `pi-<name>-<hash>-home` disk at `/home/pi`. A disk project also has
  `pi-<name>-<hash>-project`. The hash is of the canonical project root
  path. Agent settings, sessions and caches sit at their default paths in
  the home disk. The cosmetic name is at most 32 ASCII bytes.
- **Project mount:** appears at the canonical host project path. A share
  exposes host files directly and creates no project disk. A disk is created
  empty and reused across owners; Git initialization and synchronization
  follow the Git section. Linux requires a Git project. Git metadata
  directories and operational discovery errors never become non-Git projects.
  Independent nested repositories retain their own project discovery.
- **Git metadata:** For a share, pinfold reads Git metadata as data and never
  executes Git on the host. Main-root and linked-worktree topology, including
  `gitdir:` files and `commondir` in directory-form repositories, uses bounded,
  no-follow, descriptor-relative reads as the host uid. Pointers outside the
  main root’s `.git` refuse. Disk projects retain host-Git verification.
  The box stages and commits in the shared `.git` with ordinary Git.
  `pinfold -- CMD` on a share prints `git-optional-locks` and a one-line
  reason on stderr unless the caller's `GIT_OPTIONAL_LOCKS` is exactly `0`.
  Disk projects and
  `pinfold start` omit this warning.
- **Lifetime:** one detached owner serves a canonical project host-wide,
  independently of HOME and XDG roots. A session ends when its command returns
  or its client disconnects. A departed session's command group is asked to
  stop, even while a keep request or crossing lease retains the box. Processes
  that leave the command's process group, or for a terminal session ignore
  hangup after leaving it, may keep running. `pinfold stop` ends everything.
  Normal completion preserves background children. Ending one session does not end another.
  Two share sessions use one generation, one share and one home disk. Their
  command helpers retain framed stdio, cwd, TTY and exit-status behavior.
  With no session, crossing lease or keep request, the owner closes admission
  and tears down the box.
- **Startup:** after bootstrap validation, claim a fresh box name and publish
  control before runtime checks and image leasing. Existing name records,
  state or shutdown endpoints refuse. These checks run as retained cancellable
  work; catalog and image-lease locks wait nonblocking with cancellation.
  Public box-up checks and orphan recovery keep their existing order.
- **Keep/stop:** `pinfold start` keeps the project box past its last session.
  Repeated matching starts succeed. For a share, start commits keep without
  executing a command or synchronizing Git. `pinfold stop` targets the observed
  generation, works without current configuration, and succeeds only after
  teardown or confirmed absence. For a share, it revokes that box’s filesystem
  access; existing edits remain. It neither commits nor rolls back files and
  cannot stop host processes already executing box-written content. On Apple,
  owner death closes egress but may leave an orphan box able to access its
  share until checked removal. Stop from the recorded main root remains
  possible without configuration, including a hostile Git marker acquired
  during the run. It never stops a replacement. A silent owner
  has a ten-second control deadline; timeout leaves ownership intact. Shared
  maintenance ownership is acquired nonblocking within a signal-aware deadline.
  Waiting for a live launcher remains signal-cancellable without a startup
  deadline, with one waiting notice. The owner reports startup progress.
  The last client and stop wait for acknowledged, owner-bounded teardown, not
  the control deadline, and remain signal-cancellable. Startup teardown uses the
  same acknowledgement and wait. Observed command status
  retains completed-exec precedence. Teardown failure is reported as
  `teardown-failed` on stderr and in the run record.
- **Rex (macOS):** `pinfold start` asks its owner for a Rex relay when
  `REX_SESSION` is set and stdin, stdout and stderr are terminals; an owner
  off macOS answers as if `rex` were absent. The owner checks, in order:
  `rex --autostart=false version` succeeds in the box (exit status only);
  `defaults read com.superlogical.rex rex.client.remoteControl` prints `0`.
  It then runs `rex server run --listen
  unix:///home/pi/.local/state/rex/run/server.sock` in the box and waits
  until an exec of `pinfold init rex-dial` with closed stdin connects to
  that socket. It reads the preference again, and only then binds
  `<host directory>/<project key>.rex`, a host-only 0600 socket outside
  the generation directory. Each helper, the readiness wait and each
  preference read has a five-second bound; `start` waits up to 30 seconds
  for the answer. Each connection gets its own exec of `pinfold init
  rex-dial`, which copies bytes to the fixed guest socket. The owner copies
  the connection through pipes itself. The host's end reaches the guest,
  whose reply may follow for five seconds before the exec is ended; the
  guest's end ends the connection. A repeated start reuses the relay while
  its server exec runs; after that exec exits, the next start sets the
  relay up again. `start` prints on stderr `pinfold start: rex: in the Rex
  app, add unix://PATH once under Add Remote Host…` and `pinfold start:
  rex: the host is reachable while this project's box runs.` Remote Control
  on or unreadable, at either read, sets up nothing, closes a relay already
  up, and prints one `rex-remote-control` line. A failed setup prints one
  `rex-relay-failed` line. Without `rex` in the box, outside Rex or without
  terminals, `start` behaves as without this feature and prints nothing
  more. A first start's setup yields to stop, box exit, a signal and the
  client leaving. Once the owner's shutdown has begun no relay exists:
  pending setup is cancelled and new requests are refused, a setup that
  finished meanwhile is not answered, and a start waiting on it gets the
  draining answer. The relay closes, its execs end and its socket is
  removed before teardown, including when that close is itself cancelled.
  Owner death ends every connection and leaves a socket that refuses
  connections; the guest server's exec ends with the orphan box. Only an
  owner bootstrapping under the launch lock, or, on macOS, `stop` holding
  the launch lock with no project locator left, removes a left socket. The relay never starts or selects
  a box. pinfold never drives the Rex app.
- **Admission:** joining and keeping never change running policy or
  credentials. Different host roots or effective startup configuration refuse
  as `config-changed`, naming differing fields. On a known profile mismatch
  the diagnostic identifies the running profile, the requested profile and
  its source, and how to retry. Other values and fingerprints are omitted.
  Comparison uses the owner's in-memory startup snapshot, not its launch record.
  Compare host roots, profile selection, image source/reference/resolved
  identity, resources, mount and disk definitions, ordered allow entries,
  complete route declarations and sources, static referenced values, and
  environment names, literals and the values their `from` variables hold,
  so a client whose `from` variable holds another value refuses as
  `config-changed` (`env.values`). Absent and empty differ. Command
  metadata, refreshed Codex tokens, recipe freshness, and seed/shared-file
  contents are excluded. HOME is canonicalized like the other host roots.
  Mount kind and canonical source and destination paths are startup
  configuration. Shared-file contents, HEAD, refs, index contents and mtimes
  are excluded. Host and box edits and commits do not cause `config-changed`.
  Client resolution and owner re-resolution compare against the owner’s
  in-memory snapshot. For a share, the owner also checks the root `.git`
  marker without following it, on every Probe and immediately before every
  Join or Keep grant: a marker that appeared, disappeared or is not a plain
  directory refuses as `config-changed` (`repository`) while that owner
  lives. After stop, the next owner treats the folder by its current marker.
  A project admission frame is at most 16 MiB. Malformed, unsupported or
  oversized private messages fail as `session-protocol`; their contents are
  never reported. Unavailable ownership refuses as `owner-unavailable`;
  a control deadline expires as `owner-timeout`. Transport failures retain a
  fixed source (`connect`, `inspect` or `reply`) without request contents.
  Peer and recoverable accept errors reject only that connection. Queued
  requests and in-flight parsers receive `Draining` concurrently with teardown.
- **Arguments** pass through unchanged, including empty arguments and
  `--help`. Commands start at the canonical invoking directory. Trusted init
  opens the guest project root and walks the relative cwd by directory
  descriptors without following any symlink component, then enters that
  descriptor. An escape, missing directory or any symlink component,
  including a symlink within the project, fails setup. A non-UTF-8 path in
  a project box spec or configuration
  report is refused as `path-not-utf8`, naming its field.
- **Environment:** the core's proxy variables, `PINFOLD_ALLOW`, the
  configured `env` and `PINFOLD_ENV_*` (Configuration). Cwd, terminal metadata and the optional Herdr label are
  per-command execution metadata, carried through private transport, never
  environment-setting argv. They are applied after the restored box environment.
  Core and crossing commands supply no agent override.
- **TTY** whenever stdin and stdout are terminals (Process supervision).
- **Auth:** a subscription or API key through login routes (Egress proxy),
  kept on the host; else OAuth `/login` once per project, or the environment.
  A profile's login routes provide reusable host auth unless the project's
  configuration or environment replaces `routes`.
- **herdr:** no herdr socket. The TTY passes through, so screen detection
  works. Each project command sets `HERDR_AGENT` to its exact first argv
  word, including a path, overriding a forwarded value.
- **After a run:** when the box ends for a reason other than
  `sessions-ended` or `signal` (an `audit-log` stop, the box's init exiting),
  `pinfold -- CMD` prints that reason and `pinfold box inspect <run_id>` on
  stderr after teardown. An interrupted crossing reports the same reason
  and generation, and the helper fails. Stop accepted during startup reports
  `stopped`. There is no exit summary or empty-disk inspection
  or automatic deletion. CMD owns stdout; diagnostics use stderr.
  A share has no project disk, disk attachment, unfilled marker or clean/unclean
  state. Its home retains managed-disk claims and recovery rules. A share
  launch never deletes old project volumes; there is no migration or
  copy-back. After stop, ordinary `disk rm` can remove a selected old project
  disk and its managed markers while retaining home, host files, configuration,
  recipes and run records. Owner discovery requires no project disk; unknown
  runtime liveness preserves ownership and prevents replacement.

### Git

This section applies only to disk projects. Shares have no startup
synchronization or project crossing. The Git remote refuses a share as
`share-no-crossing` before acquiring a crossing lease or starting a
transient box. Before marker discovery, crossing asks recorded
share owners whose roots cover the invoking folder. A locator mount hint
selects these requests; the live owner decides. Disk ancestors neither select
the crossing root nor grant a lease for this check. Other crossing uses normal
repository discovery; linked worktrees beneath a disk project refuse. Host
roots are checked before a share owner refuses. A running share owner refuses
crossing regardless of box-written Git markers or a configuration file that now
selects disk. On macOS, a running disk owner retains disk crossing and disk path
validation when the file now selects share. On Linux, a share configuration is
invalid and refuses on every path, including crossing against a running
disk owner. With no owner, configuration selects the mount kind.

The project disk holds the box's own repository. Only the host's
branches enter it: no host `.git` content, config, hook, stash, reflog,
or untracked or ignored file, and a submodule only as its gitlink.
The agent commits there,
under the box's hooks and config, none of which reach the host. Built-in
images set git's system identity to `pi <pi@pinfold.invalid>`; a
repository or home identity overrides it.

**The Git remote.** The host repository's remote `box` has the URL
`pinfold::ROOT`, ROOT being the canonical project root. Git runs it
through `git-remote-pinfold`, a link to the pinfold executable on PATH;
invoked by that name, pinfold is the helper. `git fetch box` is
Crossing's fetch into `refs/remotes/box/*`; `git push box BRANCH` is
Crossing's push into the box's `refs/remotes/host/*`. The user merges;
signing and host hooks are the user's. Uncommitted host edits do not
cross. The helper:

- offers `fetch` (with `list`) and `push`, never `connect`, `import` or
  `option`, so Git refuses `push --atomic` and `push --dry-run`;
- refuses a remote name that is not a plain name, as Git passes a URL
  given on the command line; a URL whose ROOT is not the project Git
  runs in; a `GIT_DIR` other than ROOT's `.git`, so a linked worktree
  too; any of `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`,
  `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_CONFIG`, `GIT_NAMESPACE`,
  `GIT_REPLACE_REF_BASE`, `GIT_SHALLOW_FILE` or `GIT_GRAFT_FILE` set; a
  remote without `remote.NAME.url` in the configuration, as a legacy
  `.git/remotes` or `.git/branches` file names one; a configuration Git
  cannot read; a symbolic ref under `refs/remotes/NAME/`; and a
  configured `remote.NAME.fetch`, from a file or `git -c`, whose
  destination lies outside `refs/remotes/NAME/`;
- does the whole box exchange and every check at Git's `list`, answers
  with the repository's object format and the checked branches only, and moves the quarantine's packs in at
  Git's `fetch`; it serves one listing per process;
- pushes each `refs/heads/SRC:refs/heads/BRANCH` Git sends to the box's
  `refs/remotes/host/BRANCH`, rejecting other sources (a stash, a tag, a
  commit expression), other destinations and deletions; a box failure
  rejects every ref of that push;
- speaks Git's protocol on its own descriptors: its standard input is
  `/dev/null` and its standard output is stderr.

A refusal or failure prints `pinfold remote: DETAIL` on stderr, with any
box-controlled text escaped, and the helper exits nonzero; Git reports
it with its own exit code. A crossing refusal names `spec` or `limit`.
Both directions use a running disk owner’s box with a temporary crossing
lease, or load configuration and start a transient box on its project
disk without egress when no owner runs. With no project disk, or one
whose box never came ready (Disks), they fail and start no box.

**First start.** Project admission follows core readiness and completion
of the initial Git synchronization. A new owner runs it, a join never
does, and it acts only in the start whose `up` created the project
disk, for a Git project. A disk that start did not create (a reused one,
one `box up` created, or one first used outside Git) gets no automatic
push, whatever the box does to it, and a failed push is never retried. Explicit crossing waits for this boundary and
retains the box during transfer. Synchronization prints nothing. The box
gets a new repository, by `git init` in the box as the box user, with the host
repository's object format and its initial branch named for the host's
checked-out branch, or git's default when the host's HEAD is
detached. No step runs `git-remote-pinfold`. Unless the host repository already sets
`remote.box.url`, pinfold then writes the remote as `git remote add box
pinfold::ROOT` does: `remote.box.url` and `remote.box.fetch =
+refs/heads/*:refs/remotes/box/*`, and no other configuration or hook.
It pushes every host branch into `refs/remotes/host/*` and checks out
`host/<branch>` on the new repository's unborn branch, by `git merge
--ff-only` in the box as the box user. CMD is not yet running; the box's
own hooks apply. The push bundles every branch's full history, so it
grows with the repository, and no step has a deadline. A checkout with
no branches skips the push and the checkout silently. A failed step
skips the rest and is not retried. A failure prints `pinfold:
sync-failed: DETAIL` on stderr, naming the recovery (check `git remote
get-url box` or add the remote, then `git push box BRANCH`), and CMD
starts. Later host commits cross by `git push box`.

## Configuration

A project's configuration is
`~/.config/pinfold/projects/<id>/pinfold.toml` under the XDG rules, where
`<id>` is the project's state id (pi layer, State). The project's image
recipe, when it has one, is `Containerfile` beside it. Nothing in the
project folder configures pinfold. Setup I/O is interactive when stdin,
stdout and stderr are terminals and stdin's is already in canonical mode;
pinfold never changes its mode for a prompt. When the file is missing and
I/O is interactive, `-- CMD` offers a profile, initially `default`.
A set `PINFOLD_PROFILE` selects the profile and is identified in the prompt.
Setup identifies whether the selected profile is bundled or user-defined,
names the project root and describes the platform's mount. Confirmation saves the
profile and mount and continues with the original command and arguments.
Refusal or EOF before confirmation exits 125 without creating the file.
If another caller creates the file while the prompt is open, this launch
refuses and leaves that file in place. Without interactive I/O, `-- CMD` refuses
as `no-config` and leaves stdin untouched. `start`, `build`, `config` and
the Git remote also refuse as `no-config`. The diagnostic names the interactive
`pinfold -- pi` and `pinfold -- sh` setup commands.
`doctor` reports it as a finding. An unreadable file
or a dangling symlink fails. `pinfold config edit` checks a project as
below, with the profile a run would select (a profile that cannot be
loaded adds no paths, so the editor still opens), creates the file in one step when it is missing, holding only the
project mount below, and opens it in `$VISUAL`, else `$EDITOR`, else
`vi`; it exits with the editor's code. On Linux every project command
outside Git, `config edit` included, is refused as `not-git`.

The project mount is required. `config edit` creates
`mounts = [{ host = ".", kind = "share" }]` on macOS and
`mounts = [{ host = ".", kind = "disk" }]` on Linux. macOS accepts either
kind; Linux accepts only `disk`. Other project mount shapes refuse as `spec`.
This does not add arbitrary extra project mounts.

A project is refused as `host-path-in-project` when any of the protected
paths listed below resolves inside it. This check applies before project
state or configuration is created, whether or not the project is a Git
checkout.

Protected paths are pinfold's configuration, state or cache root, the
project's configuration directory, file or Containerfile, the directory of
the selected profile and of each base in its chain, their `pinfold.toml`,
`Containerfile`, `home` or `share`, the selected profile's share mount,
`artifacts/` and the lock directory. Resolution follows symlinked ancestors
too, including missing suffixes and a dangling profile share symlink:
box content crosses into the project and must not reach them.
`-- CMD`, `start`, the Git remote, `build` and `config edit` check it, and `doctor`
reports it as a finding. Project clients perform admission checks before
automatic maintenance or notices. Other working commands may already have
run the daily maintenance pass and written under the state root before
this check.

Layers: environment > the project's file > the profile's `pinfold.toml` >
each base's `pinfold.toml`, nearest first > built-in defaults. Each key takes the highest layer that sets it, lists
included: a list replaces the ones below it, and `allow = []` allows
nothing. An environment variable that is set, even empty, sets its key.
An override containing non-UTF-8 bytes is refused, naming only its key.
Only the environment and the project's file select `profile`; a profile's
own `profile` value is ignored. `env` is the exception to whole-key
replacement: each layer replaces it name by name, so a name a lower layer
sets cannot be unset above it. A configured `HOME` is ignored:
the project box sets it, as it does the always-applied names, and
`config --json` omits it.

A `from` value is read once, when a new box starts. A missing variable
refuses the start before any owner starts, and so does a missing injecting
or login route variable: the client prints `environment variable VAR is
not set` on stderr and exits 125. The value travels as a `PINFOLD_ENV_`
value does: in the environment of the detached owner and its runtime
client, and, for the join comparison, in the admission snapshot, held in
memory and sent over the private project socket. Argv,
`config --json`, the launch record and every file pinfold writes name the
variable, never its value. pinfold passes the runtime no box variable
(Always applied). A literal is configuration, not a secret:
`config --json` shows it; the launch record names it, like every box
variable, without its value. Secrets belong in `from`. Always-applied names follow the
box spec's rules (Always applied).

| Key | Env | Default | Meaning |
|---|---|---|---|
| `profile` | `PINFOLD_PROFILE` | `default` | Profile name |
| `base` | — | — | The profile this one builds on (Profiles); profile file only |
| `image` | — | — | Local image reference, independent of the profile; project overrides profile |
| `allow` | `PINFOLD_ALLOW` | `api.anthropic.com`, `platform.claude.com`, `api.openai.com`, `auth.openai.com`, `chatgpt.com`, `openrouter.ai`, `opencode.ai`, `registry.npmjs.org`, `pi.dev` | Hosts the proxy lets through |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes: `name = "host:port"` for a host service, an injecting route `name = { to = "https://…", headers = { Authorization = { from = "VAR", prefix = "Bearer " } } }`, or a pi login route `name = { login = "pi", provider = "openrouter", from = "OPENROUTER_API_KEY" }` (see Egress proxy) |
| `mounts` | — | — | The project's mounts (above); project file only |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| `env` | — | `{}` | The box's variables: `NAME = "literal"`, or `NAME = { from = "VAR" }` read from the caller's environment at start; the box spec's `env` (Process interface) |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box from the caller's value, over `env`'s `NAME` |
| — | `PINFOLD_NO_UPDATE_CHECK` | — | `1` disables interactive update checks and notices (Updates) |

The list keys take comma-separated values in the environment:

- `PINFOLD_ALLOW`: host names; a leading `.` makes a suffix entry.
- `PINFOLD_ROUTES`: `name=host:port` pairs; an injecting or login route needs a file.

An unknown key in the project's file or a profile's `pinfold.toml` is
refused, naming the file and the key. `mounts` in a profile's file and
`base` in the project's file are refused.

`pinfold config` summarizes the project folder, mount, selected profile and its
source, image and its availability, and settings path. It reads configuration
without changing state, even when the runtime is unavailable.
`pinfold config --json` prints the effective configuration. `env` maps each
box variable to its literal or its `{"from": VAR}`, a `PINFOLD_ENV_<NAME>`
as `{"from": "PINFOLD_ENV_<NAME>"}`. Its `project` reports its id, home path
and `mount`, containing `kind`, canonical `host` and `guest`.
`project.disks.home` names the persistent home disk. `project.disks.project`
names the disk for a disk project and is null for a share. `config` is the
project’s configuration file. `image_built` is true or false when
the runtime image inventory succeeds, and null when it cannot be observed;
`image_error` names that failure. Host configuration facts remain
available. `image` is the selected reference; `image_source` is `explicit`,
`project` or `profile`. `containerfile` is null for an explicit image,
the project recipe's path, or `profile` for a profile recipe. `image_built` reports local presence, not managed provenance or
recipe freshness. Launch records retain the resolved image id and reference
(Run records).

`start` checks local image availability before recording project state, as
`-- CMD` does. Joining does not pull or build an image.

## CLI

```
pinfold -- CMD [args…]           start or join a project session
pinfold start                   keep this project's box
pinfold stop                    stop its observed box
pinfold build [--profile NAME] [--no-cache] [--allow HOST]…   build this project's image, or a profile's; prints the ref
pinfold image build|rm NAME … [--json]   a caller's image, built from its own context, or retired (Process interface)
pinfold profile new NAME [--from PROFILE]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim unused disk space
pinfold doctor [--artifacts] [--json]   diagnose this host and project, or list artifact pins
pinfold apparmor-profile         the AppArmor profile for this executable (Linux)
pinfold update [--check]         check for a release, or download and install it
pinfold config [ROOT] [--json]   selected project settings and image
pinfold config edit              create this project's configuration file if missing, and open it in $VISUAL or $EDITOR
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
git fetch box / git push box     a disk project's branches, through git-remote-pinfold (Git)
```

`doctor --artifacts --json` prints one JSON array: the host `codex` helper, then on Linux
entries named `vmm` and `builder`: `name`, `version`, `path` (the helper
executable or infrastructure install directory), `cached`, and
`assets`, a list of `{url, sha256}` for this host's `os-arch`. Nothing is
downloaded. The builder asset includes `url`, `sha256` and `bytes`; its
entry also reports the recipe identity. Without `--json`, `doctor --artifacts`
prints a readable inventory. Plain `doctor` leads with findings and remedies.
`doctor --json` prints the diagnostic report as structured data.

`pinfold --version` (`-V`) and `pinfold --help` (`-h`) answer without
touching the runtime or the state directory. Bare `pinfold` reports the
project root, box status, profile, project mount, image and settings path.
While a box runs, it also reports that box's mount and labels the
configured profile, mount and image as next-launch settings.
With no configuration it previews interactive setup, including whether
the selected image can be built. It reads project and owner information
without creating state, starting a box or querying the container runtime.
Unavailable details are identified. It shows `pinfold -- CMD` and succeeds.
Explicit help groups the command inventory
by task. Report commands use human output unless `--json` is requested,
including when stdout is redirected. The box protocol and guest stdio retain
their own output contracts. Unknown top-level commands name the
command and point to help. Editor failure reports the status and how to open
the configuration file. `--help` or `-h` after a subcommand, before any
`--`, prints that subcommand's syntax line and exits 0, touching neither.
The first top-level `--` introduces literal command argv; subsequent
arguments are never interpreted as pinfold help. Bare `pinfold --` or an
empty command name exits 125 before project setup. Arguments must be UTF-8.
Host paths in human diagnostics are control-character escaped; JSON uses
normal JSON serialization.

`start` and `stop` take no arguments and print nothing to stdout on success.
They return 0 on success or 125 for a project infrastructure failure; host
signals retain `128+n`. `stop` and the private owner entrypoint run no automatic
maintenance or update notices. Public core and crossing exit conventions are
unchanged.

A project is a main Git checkout’s canonical root, including its linked
worktrees beneath that root, else the canonical invoking folder. A linked
worktree is admitted only when its common Git directory is that main root’s
`.git`. External linked worktrees and separate Git directories are
refused. A moved or renamed project has a new identity and no configuration.
Discovery reads metadata only. No configuration is parsed before a live
owner compares host roots; changed roots refuse as `config-changed` before
configuration or worktree errors. For a share, topology is established by the
Git-metadata control above. Independent nested repositories retain their own
project discovery. Inside a Git directory, the command refuses as `project-in-git`.

## Guarantees

The end-to-end proofs are suspended (`AGENTS.md`, Tests): the suite is
removed, and no row is proven until it is rewritten. "Shown by" states
what the rewritten suite must show. Testing policy is in `AGENTS.md`.

- 34 is a slow gate outside the five-minute suite, the Slow response proof
  workflow. Release publication, including nightly pin releases, requires
  its success at the release's exact ref. It fails while the suite is
  removed, so no release publishes.
- 36 is the Linux slow gate `scripts/slow-proxy.sh`, run by manual dispatch.

| # | Guarantee | Shown by |
|---|---|---|
| 1 | No network but loopback | Inside: only `lo`; `1.1.1.1` unreachable. Without `egress`, nothing gets out, not even through a route: a request to the proxy's port is refused, and on Linux a vsock connection to the host proxy's port is reset. Control: with a route, the fixture answers through the proxy. On Linux, from inside, a vsock connection to CID 2 on the exec port and on an unused port is reset at once, not timed out; control: CID 2 port 3128 reaches the proxy. The guest kernel's `/proc/config.gz` leaves vsock loopback unset. |
| 2 | Only allowlisted hosts get through | `api.github.com` answers. `example.com` gets a proxy 403, logged "not allowlisted"; `ready` names that log's path. Each trick has a positive control in the same box: IP literal, loopback resolution, SNI mismatch, CONNECT to a route and ambiguous Content-Length framing are refused, each logged with its own reason. A raw valid Content-Length POST delivers its body to the same host fixture. |
| 4 | A route reaches exactly one host service | A route reaches the fixture with the Host header rewritten from the client's and logs method, queryless path and upstream status. Replacing its writable audit log with a directory makes the next request stop the owner with audit-log, exit 1 and, on Apple, no runtime box. |
| 5 | Nothing can gain privileges | `CapBnd` 0 and the host uid in exec'd processes; an exec workload's fork+exec descendant reports `NoNewPrivs` 1; no setuid or setgid files; rootfs not writable; init's credentials are the host uid and its `NoNewPrivs` is 1 on Apple; on Linux PID 1 is root with `CapBnd` exactly `CAP_SYS_ADMIN`, the exec service is the host uid with every capability set empty and box work cannot list its `/proc/<pid>/fd`. On both runtimes `user.max_user_namespaces` is 0, a descendant's `unshare -U` fails with ENOSPC, and a plain child succeeds in the same box. The box's `uname -r` differs from the host's; on Linux it is the guest kernel `vmm.toml` pins. Share-inspecting initialization, execution and session helpers use the host uid and retain the stated privilege restrictions. |
| 6 | The environment follows the spec and always-applied values | A caller box whose `up` has unprefixed proxy variables does not get them. Guest PATH, HOME, `XDG_RUNTIME_DIR`, `LD_PRELOAD`, multiline values and a transport-prefixed name arrive exactly while local list/exec/down still manage the box. Always-applied values win. Apple shares append exact-root Git ownership entries through `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_n` and `GIT_CONFIG_VALUE_n`, preserving declared Git entries. PINFOLD_ALLOW defaults to the spec allowlist without routes and an explicit spec value wins. Guarantee 39 shows PINFOLD_ENV_SECRET's path. On Apple no box variable is passed as environment through `container run`: while the box runs, neither `container inspect` nor any file in Apple's container directory for the box holds a `from` or `PINFOLD_ENV_` value, plain or hex. A literal `HOME` is also a path: it places generated mounts, such as a Pi login route's, so it appears in their `container run --mount`, Apple's configuration and the launch record, and a refusal of such a mount may name it. A literal holding NUL is refused as `spec`, naming only the variable. |
| 8 | Losing the owner fails closed | After SIGKILL of `box up`, egress fails closed; another XDG root sees the owner dead, prunes the box and reuses its name. The run's launch record stays `available` and its terminal `not-observed`, also after the prune. An abandoned partial claim is reclaimable. On Linux the VM ends with its owner (guarantee 43), so the egress half runs on Apple. Repeat owner death with a detached project owner, including a kept owner without clients. Surviving clients report 125 unless they already observed command completion. Prune and restart preserve the missing terminal record and existing dirty-disk recovery rules. Owner death during gated startup or retirement remains recoverable, including after identity deletion and before claim unlink. An Apple orphan may retain share access until checked removal; replacement waits for removal and retirement. |
| 9 | The lifecycle works for a caller | Before startup completes, public `exec` exits 3 without starting a caller command; after `ready`, the same command succeeds. `exec` immediately after `ready` streams both streams and returns the exit code, and exits 3 on an absent box; `ready` and `list` name the image's id, and without egress `ready`'s `egress_log` is null; `down` of an absent, empty or flag-like name exits 0, prints nothing and leaves the live box alone; closing `up`'s stdin tears the box down with reason `stdin-closed`. A real terminal exec preserves the caller's input file flags, whether initially blocking or nonblocking. On Apple, a container pinfold did not create is absent to `list`, `exec`, `stat` and `down`. |
| 10 | Disk-project branches cross only through the Git remote, and end at refs | For disk projects, a box’s writes on its disk leave the host project's bytes, modes and mtimes unchanged. A populated copied Git tree has sampled caller uid/gid ownership, no effective `safe.directory` exemption, and a clean ordinary-user status before mutation. `git fetch box` lands only the box's branches, under `refs/remotes/box/*`, and no tag, two that shadow a branch's names included; after a merge and two checkouts on the host, nothing the box planted (hooks under every client-side name, `core.hooksPath`, `core.fsmonitor`, tracked `.githooks/` and `.husky/`) has run; control: a host hook runs. From a box that sends its own bytes, each of five hostile bundles, a bundle past 4 GiB, a symref or tag line, a malformed or control-character branch name, more than 4096 branches, a `.git` tree entry, a missing tree, a filtered bundle, a listed branch at a commit `box/*` does not reach, the same with a bundle that names that host commit as its head, a branch at an annotated tag, a bundle head the listing does not name, a listed branch at a commit the pack carries but the bundle does not name, and an object id of the other hash's width fails the fetch naming its cause and changes no host ref and nothing under `.git/objects`; control: a listed branch at a reached commit lands. With `fetch.prune` and `fetch.pruneTags` set, a bundle carrying a tag leaves every host tag as it was. A configured fetch refspec outside `refs/remotes/box/`, from the file or `git -c`, a `pinfold::` URL on the command line, a remote named only by `.git/remotes/box`, a symbolic ref under `refs/remotes/box/`, and `GIT_OBJECT_DIRECTORY` or `GIT_CONFIG` in the environment are refused the same way. SIGKILL of the helper during the box's stream or after `list` changes no ref; the next fetch removes the left quarantine, and against a transient box starts on the disk the dead box left unclean and leaves it clean. Two concurrent fetches of the same commits both succeed and leave a repository fsck accepts. `git push box BRANCH` lands in the box's `refs/remotes/host/*` only; a tag destination, a deletion and a stash or tag source are rejected, and a failed push prints none of the box's stderr. |
| 11 | Disks persist and attach exclusively | A disk created with `from` holds the host tree's files, exec bits and symlink; after its owner is SIGKILLed during the copy and its box pruned, the next `up` naming it holds the whole tree. A file one box writes is read by the next box that attaches it; another state root naming the same disk gets its own. While a box attaches a disk writable, another `up` naming it and `disk rm` (including `--all`) are refused as `disk-in-use` and create nothing; of two writable `up`s racing for one new disk exactly one runs and the other is refused as `disk-in-use`; two boxes attaching a disk read-only cannot write it or a read-only host mount. A sparse `from` past the disk's `size` is refused as `limit`. After the runtime kills a writer's box, or the writer's owner is SIGKILLed, a read-only attachment is refused as `disk-unclean` until a writable one has run, and then reads the disk. On macOS, a dead owner's box the runtime created but never started holds its disk. `disk rm ../escape` is refused as `spec`; after writers stop, `disk rm --all` removes the remaining disks. On Linux, a nested disk whose mount point is a symlink the box planted ends the start `failed` and leaves nothing at the link's target. A fresh home whose source contains `.pi/agent/settings.json` starts with a Pi login route and retains those settings; a reused home ignores changed initial-source content. On both platforms, generated parents on a fresh or reused home are created as the numeric box user through descriptor-relative no-follow traversal; an existing symlink fails startup without writing through it, and its corrected directory succeeds without changing existing ownership. |
| 13 | Project state persists and stays separate | Nested Git metadata and bare-repository paths refuse as `project-in-git`; a linked worktree beneath a shared main root uses that root’s configuration and owner at its invoking cwd; external worktrees and separate Git directories refuse before project state. The main checkout runs. State, config or cache roots inside the project, reached through a symlinked ancestor whose suffix does not exist, and the project's configuration file, directory or Containerfile or the profile's `pinfold.toml` symlinked into the checkout, make build and project commands refuse as `host-path-in-project` and create no project state; `config edit` refuses the selected profile's directory symlinked into the checkout, whether `PINFOLD_PROFILE` or the existing file selects it, and writes no file; with external roots the same project runs. Guest writes in two same-named projects remain separate on A/B/A runs. For disk projects, the first start’s box repository is checked out clean at the host's HEAD commit, on a branch two tags shadow by its short and its full name, holding its committed file, and holds none of the host's untracked `.env`, unstaged blob, stash, remote URL or hook. For disk projects, with no identity in the repository or home, the box's commits arrive authored `pi <pi@pinfold.invalid>`. For disk projects, the first start adds the host remote `box` at `pinfold::ROOT`, with no `git-remote-pinfold` on PATH, leaves an existing `remote.box.url` as it was, writes no other configuration, and gives a sha256 host repository a sha256 box repository; a later owner does not push, so a host commit made after the first start leaves the box's `host/main` where it was, also after the box deletes its `.git`; a project disk `box up` created, or first used before `git init`, gets no push. A non-Git macOS share exposes existing host files and retains its home across owners; another canonical folder has a separate owner and home. These folders receive the same protected-path checks and project maintenance ownership as Git projects. Each protected-path refusal creates no project state or configuration, and external roots permit the same project. On Linux the non-Git folder is refused as `not-git`; a configured Git checkout runs. A long valid project basename launches. For disk projects, joining does not push, and project crossing cannot enter between core readiness and completed initial synchronization. |
| 15 | Cleanup removes only pinfold's garbage | One project image supplies a live box alongside an idle project's state and disk. Both projects' state bytes and disks survive `clean --dry-run --unused 0s`; plain `clean` keeps both disks. Real `clean --unused 0s` removes idle state and disk while preserving live state bytes and a write read back through the live box's disk. On Apple an externally started container and an unlabeled external image survive cleanup. One caller image held by a running box survives `image rm`, reports `in_use` and still serves its fixture bytes; after the box stops the same removal reclaims its refs and, on Linux, its observed root disk. A kept project with no clients retains its state bytes, disks and readable guest writes through cleanup; after stop, the same eligible cleanup removes them. Share-project cleanup never removes host shared files. |
| 17 | up refuses before it creates | A missing image, a misspelled nested spec key, a bad env name, a `dev.pinfold.` label, case-colliding route names, a suffix allow entry on or above a public suffix, a mount or disk `from` path with a comma, a disk at or below `/proc`, `/sys` or `/dev` or at `/`, `/tmp`, `/etc` or `/etc/resolv.conf`, each reached through `..` or `.`, a flag- or path-like disk name and a live name are refused as data, naming the cause; the misspelled key's refusal also names the box; each leaves no box and no state dir. Pi login HOME without an absolute writable-disk backing, overlapping generated destinations and Linux attachment 15 refuse before preparation; corrected HOME/mounts and Linux attachment 14 run. A recipe-less profile refuses with `image-source-missing` even when its old managed tag exists; the same profile with an explicit image runs. Of two `up`s racing for one name exactly one wins, and the winner answers exec and reaches each distinct route name. Long valid XDG state roots permit startup, exec, stat, shutdown, restart and orphan recovery on both runtimes. An unsafe socket path needed by a new generation refuses as runtime-path without changing its target; a checked private directory permits the same start. After confirmed runtime removal, an unverifiable old generation directory remains unchanged while its dead claim retires and a fresh generation starts. A Linux share refuses as `spec` before write-producing startup work, with absent pinfold roots remaining absent; a supported mount succeeds. |
| 18 | A caller reads the effective configuration as data | Without a runtime, `config --json` reports host policy without creating state/cache/config files. Linux reports known availability from its local catalog, including false for an absent catalog; Apple reports image_built null and image_error when its native probe is unavailable. An existing Linux image remains reportable with empty PATH and KVM denied, without changing the catalog. A project with no configuration file, a `.pinfold.toml` in its folder included, makes config, build and project commands refuse as `no-config` and create no project state; `config edit` with `EDITOR=true` writes the file, and the same commands proceed; `config edit` still opens the editor when the file names a missing profile; a moved copy of the project is refused again. Unsupported project mount shapes and Linux shares refuse as `spec`; a dangling configuration symlink is refused, and `config edit` writes nothing through it. Unknown project/profile TOML keys and a profile's `mounts` are refused with the source and key; correcting the same file restores the fixture policy. With `PINFOLD_PROFILE=adam` and no project `allow`, `config --json` reports the default list plus `github.com` and `api.github.com`. Layered image defaults and project overrides report `image_source` and null recipe for explicit images, and `build` refuses an explicit image as `image-no-recipe`; a project recipe works with a recipe-less profile. Conflicting, missing and empty sources refuse with their tokens before box/project/disk/run state; correcting the fixture succeeds. After a managed rebuild, config reports the selected image reference, source and availability. `project` reports its id, home path and `mount`, containing `kind`, canonical `host` and `guest`. `project.disks.home` names the persistent home disk. `project.disks.project` names the disk for a disk project and is null for a share. Guarantee 39 shows the live report. |
| 20 | A caller can tell an OOM kill from a failure | On both runtimes stat reports the limits in force, every field is present, and `exec`'d processes start with `oom_score_adj` 1000. On Linux, the guest kernel's `MemTotal` is within 64 MiB under the box's memory. In one process and its fork+exec descendant, -999 is allowed, modern -1000 and legacy -17 return EACCES with unchanged scores, and legacy 15 succeeds. The service parent stays -1000. A sole workload at -999 touches a finite 1 GiB mapping in the 256 MiB box, exits 137, and stat's oom_kills rises; a fresh probe succeeds afterward. |
| 21 | An injecting route keeps the credential on the host | The fixture behind an injecting route receives the header; the box's environment and the egress log never hold the value; an https route reaches api.github.com over TLS. On Apple, while the box runs, neither `container inspect` nor any file in Apple's container directory for the box holds the header value or a box env value, plain or hex. |
| 23 | A caller builds an image from its own tree | Caller builds A and changed B report distinct refs, runtime-observed ids and the fixture label. After B moves latest, A's unique ref launches with A's COPYed bytes and latest launches with B's bytes; on Linux `ready` carries the image label. Linux A retains its successful outcome and both refs after an injected post-commit directory-sync EIO. Two unchanged managed builds rerun release discovery, cache installation and apt steps, and keep the image identity. A changed resolved release reinstalls agents with the apt layer cached. Managed --no-cache reruns every step. An unchanged adam-profile project rebuild keeps every image identity in its chain and launches without image-outdated. A project build rebuilds its selected profile first and records that resulting identity; failed profile installation keeps its refs, and failed project installation keeps project refs while reporting any published profile. Edited home seeds survive rebuilds. Caller builds retain layer caching and their no-cache option. Ordinary failed build F prints a tail bounded to 64 KiB after a 200,000-byte unterminated diagnostic, keeps the caller's final marker and leaves every caller ref-to-id mapping unchanged. |
| 26 | Image agents use route-generated configuration and keep logins on the host | Claude, Codex and Pi coexist in one box and complete fixture inference/tool exchanges through route-generated files, including a child-agent launch with no per-exec agent env. Pi autoloads only its mounted subdirectory alongside user extensions. Every Pi provider works. User, project, stored-login and explicit configuration precedence remains observable. The upstream receives only host-supplied credentials through the named authentication channels. Fixtures cover claude, codex and all five pi forms: `openrouter`, `openai`, `anthropic`, `anthropic-subscription` and `openai-codex`. Mixed-case duplicates of every removed field and repeated `Connection` fields nominate authentication and metadata during a nonempty POST. Each fixture receives exactly the host authentication, no removed fields, unchanged remaining metadata and the exact body. Every forbidden query name, encoded and case variants, duplicates, empty values, fields without `=` and both separators get 403 / `login credential query` with no upstream request. Malformed percent escapes and `Connection` options get 400 / `malformed request`; nominated `Content-Length` gets 400 / `ambiguous framing`. Corrected requests succeed in the same box, preserving harmless encoded queries, `anthropic-version`, `anthropic-beta` and arbitrary non-model paths unless nominated. Generic injecting and host-service routes retain their authentication and query behavior. Codex and pi share credentials after refresh within 5 minutes of expiry; a lapsed token is refused with a valid-token control. Across two XDG roots on one `CODEX_HOME`, the second refresh reports `login-busy`. Cancelling `up` during a stalled refresh reaps the helper and closes its refresh connection. `pi -p` through the image Pi and the `openrouter` route sends the profile's and the project's skills and the profile's allowlist. No host token, key or account id is in the box's environment or files, its home disk, generated state, diagnostics or egress log; a deliberately reflecting upstream can return them. Misconfigured routes are refused as `spec` or `login`. On macOS only, with a real login, codex completes one tool round trip through a route with no `to`. |
| 27 | Doctor reports without changing state | With and without a runtime and a project configuration, doctor leaves complete Pinfold-owned state/config/cache trees unchanged, including absent roots, and reports host configuration, and `no-config` and `host-path-in-project` as findings; so do `pinfold --version`, which prints the version, and `pinfold box list --help` with no runtime. A newline-containing project or protected path is escaped in doctor and refusal diagnostics and cannot forge a new structured line; correcting the configuration permits the same project to run. Bare help creates no pinfold state. Native runtime bookkeeping is outside those trees. Linux missing-kvm diagnostics use real failures. The Linux host keeps AppArmor's unprivileged user-namespace restriction on (`kernel.apparmor_restrict_unprivileged_userns` is 1); there a copy of pinfold outside its profile's path reports `userns-apparmor` and is refused as `runtime`; the profiled executable starts a box. |
| 28 | Host updates are verified and atomic | A host HTTPS release fixture supplies a different valid executable. Wrong checksums preserve the original bytes/inode; a working command started during a held asset transfer also prevents replacement. After teardown the verified bytes replace the inode and preserve the invoked command symlink. A detached owner kept without clients prevents replacement with update-busy; after stop, the same verified replacement succeeds. |
| 34 | Blocked route writes release the upstream | A small routed POST and response succeed. In concurrent connections, a guest stops reading a response and a host fixture stops reading an upload while sending a response. Both fixture connections close after the production write deadline while the guest holders remain alive. The upload producer's completion is timed independently to exclude an early failure followed by a read timeout. |
| 36 | CONNECT tunnels share activity | Initially idle TLS tunnels start at staggered times and expire after the production idle interval. Real TLS tunnels carry upload-only and download-only traffic for longer than the production idle deadline. Neither closes while bytes flow; both then close after a full idle interval with `idle timeout` in the egress log while the guest holders remain alive. |
| 38 | Each run's history is its own | Two `up`s of one name in sequence; the first makes an allowed and a refused request, and when `box down` returns, `inspect` already reports its terminal `signal`. The second's `ready.run_id` equals its `dev.pinfold.generation` label and differs from the first's, and its `ready.egress_log` is empty before its first request. Its 150 decisions, paged at the default limit while its box appends 50 more after the first page, return every line once, in order, across `next`; `events` for the first run still returns exactly its two decisions. Each `down` names its own run_id. A no-egress box SIGKILLed from the host (its init through Apple's runtime, its VMM on Linux) makes `up` exit 1 with `down` reason `exited`, and terminal `exited` whose detail names exit status 137. With no runtime on `PATH`, `inspect` and `events` answer for finished runs, and the no-egress run reports egress `not-applicable`. A malformed run id, `--limit 101` and another run's cursor exit 1; an unknown run id exits 3. A project's last unkept departure records sessions-ended with null detail before its final leave completes. Project stop records signal before returning. A replacement has a distinct generation. |
| 39 | The launch record is the applied policy | A project command starts with an explicit local image and a recipe-less profile, from its configuration file with an allow list, cpus, memory, a host-service route and an injecting route whose header comes from a host variable, plus `PINFOLD_ENV_SECRET`, under environment overrides of the allow list, cpus and memory. Its profile, on a base, sets an `env` literal, a `from` variable TOKEN and a name the project file also sets. TOKEN, the literal and the project's value arrive; `PINFOLD_ENV_` of the project's name wins over both; with the variable unset the start refuses on stderr naming it, as with an unset route header variable. `config --json` reports the literal and TOKEN's variable, never its value. `config --json` reports the file's list, the built image and the home disk; the environment's `PINFOLD_ROUTES` replaces the file's routes; an invalid UTF-8 override fails without disclosing its bytes. The box attaches the reported home disk writable at `$HOME`; its kernel sees the environment's cpus and memory; SECRET arrives without entering host argv, and on Linux is in neither `/proc/cmdline`, the VM configuration, the init disk nor any disk image. Its seeds are readable and its read-only profile files refuse guest writes without changing host bytes. While it runs, the file's image and allow list change: `config --json` reports the change, but `inspect` still reports the started allow list, route kinds and targets, cpus and memory, equal to what the test wrote, plus env names that include `SECRET`, no harness version field, and the started image reference and runtime-observed id; the started list decides, refusing the file's hosts as `not allowlisted`. No file under the state, config or cache root holds the secret, TOKEN's value or the header value, in plain or hex form, and no host process argv holds them. A second client whose TOKEN variable holds another value refuses as `config-changed` naming `env.values`; the same value joins. When the last unkept lease ends, the terminal record is `sessions-ended` with null detail, without argv, output or session statuses. On a second start, the guest-edited seed survives and current shared bytes appear. Literal argv retains empty arguments, spaces, shell metacharacters and `--help`; binary stdin/stdout and separate stderr pass through. Exits 0, 23, 125, 126 and 127 pass through, as do signal statuses `128+n`. Project setup failures, including cwd, terminal and executable lookup failures before CMD starts, use 125. External box down, cleanup or shutdown accepted during startup before an observed command completion uses 125; a signal received by pinfold itself uses 128+n. A completed exec result is checked before a ready shutdown or signal, so its observed status is retained. An observed command status survives leave or teardown failure, including after command success; the failure is reported on stderr and in the run record. Public box exec retains its codes. Nested canonical cwd matches the invoking host folder. A guest-planted symlink in that cwd refuses as setup failure, including a symlink within the project; a plain directory succeeds. An execute-only project subdirectory works as cwd through search-only descriptors, while symlinks still refuse. A real host subdirectory in a share is available as cwd; a directory absent from a disk refuses as setup failure. For disk projects, fresh project disks have no empty formatter lost+found or related Git warnings. `HERDR_AGENT` is the exact first argv word, including an executable path, and wins over a conflicting forwarded value. Separate fast Apple TTY commands inspect host dimensions and TERM/COLORTERM at their first instruction, before any resize; interactive resize then follows the host. Non-TTY execution is the control. Public forced-TTY exec with redirected stdout succeeds without a host size; zero dimensions are not sent. Host raw setup failure dispatches no command, and guest terminal setup failure is refused. Linux interrupted foreground TTY execution restores owned termios. A non-raw command with terminal stdin and redirected stdout preserves a deliberate later host stty change. Restoration checks foreground ownership before changing termios. An interactive first run saves the chosen profile and mount on confirmation. With a missing managed image, confirming the build starts the original command with its literal argv. Declining setup leaves the configuration absent. Declining a build leaves the command unstarted. Redirected no-config, missing-image and malformed-command refusals each have a corrected invocation that runs; discovery, configuration, protected paths and local image availability are validated before project state is recorded. On Apple and Linux, foreground handoff followed by client SIGTERM preserves the new foreground group's full terminal settings; the signalled client exits 143. Each joined command retains these argv, cwd, environment, terminal and status properties. The recorded mount kinds match the applied attachments. |
| 42 | Run records are retained within bounds | With inactive runs padded on the host past 128 MiB in valid decision lines, `clean` removes the oldest inactive run, keeps the newest, and leaves a live run's log unchanged. Once the newest is aged 15 days on the host, `clean` removes it (`inspect` exits 3), while a live run aged the same stays `available` and its log keeps appending. With that live log padded to 128 MiB minus one byte, the next real route request stops the owner with `audit-log`, exit 1, and leaves the log length unchanged. A kept owner without clients still protects its run from retention cleanup. Audit failure stops that owner and incomplete project clients return 125; completed command status retains precedence. An append failure as the last session leaves or during runtime removal survives proxy joining: the terminal record and final client notification name `audit-log`, while observed command codes 0 and 23 remain unchanged. A writable-log departure records `sessions-ended`. |
| 43 | The VMM reaches only its own box | Linux only. With two boxes up, each VMM is a child of its box owner, including a detached project owner, and runs as the user with `CapBnd` 0 and `NoNewPrivs` 1; its user, mount, PID, network, IPC and UTS namespaces differ from the owner's; its root, read through `/proc/PID/root`, holds its own disk and not the other box's disk; none of its mount points is under the user's home; its fds 0–2 are `/dev/null` and none is under the home; SIGKILL of the owner ends the VM. Its jail exposes neither project session nor shutdown control sockets. |

| 44 | Linux image work stays in its own VM | During an audited RUN, SIGINT ends the build with exit 1 and one failed line whose log names `cancelled`; its VMM ends and public refs stay unchanged. The next ordinary build succeeds in the same cache. Real RUN observes the approved rootful OCI role, no_new_privs, disabled user namespaces and only loopback. In the same build, api.github.com succeeds and example.com gets proxy 403 with its denial audited; explicitly admitting example.com permits it. Cache-mount exhaustion fails with unchanged public refs; its next ordinary build succeeds without manual cache cleanup. Roots, aggregate OCI payloads and selected local-base inputs above the former 512 MiB limit build and launch within their separate bounds. Exceeding each enlarged output bound fails without publication; its smaller control succeeds. Caller context retains its separate bound. After clean removes the idle cache and the pinned builder is unavailable, the completed above-old-limit image still launches and reads its fixture. |

| 45 | Files leave a disk without Git only by export into a new destination | While its box runs, export of its disk, a project disk included, refuses as disk-in-use. After teardown, or `pinfold stop` for a project, the same disk exports nested file bytes, executable mode and a symlink as a link without touching its target. Existing destinations refuse unchanged; a missing disk refuses as disk-missing and creates no disk. A special file refuses and leaves no destination; removing it makes the same disk export successfully. |

| 47 | A project's sessions share one box | Simultaneous first launches use one owner and generation. Independent clients run in distinct invoking directories. Killing or disconnecting a client sends TERM to its command group, then KILL after two seconds if needed, while the second session keeps working. Keep and crossing leases retain the box but not a departed command group. A terminal departure hangs up its guest terminal first. A process that leaves the command group, or for a terminal session ignores hangup after leaving it, may keep running; stop ends everything. Normal completion preserves background children, with output delivered for up to one second. Plain stdin EOF does not cancel a command. The final unkept departure ends the box. Bootstrap death and final departure racing join, keep and stop never admit two writable generations. Start retains a box without clients; repeated start and stop succeed. Different effective configuration or host roots refuse as config-changed without executing CMD; restoring each changed input joins the same generation. The original proxy policy still decides. A stale stop cannot end a replacement. Two share sessions edit the same host files. Host and box edits and commits permit joining; mount-kind changes refuse as `config-changed`. A gained root `.git` refuses as `config-changed` only while that owner lives; a later owner runs it as a Git project without host Git. Start creates no project disk or Git synchronization. Share crossing refuses before a lease or transient box. Stop ends box access and preserves host edits, including from the recorded main root without configuration after a hostile Git marker appears. |

| 48 | A share confines direct writes to paths in its declared host directory | On Apple, ordinary shared-file edits reach the host, and a box `git add` and `git commit` land on the host's branch with the host's prior commit as parent and the staged tree, from the main checkout and from a linked worktree beneath the root. A share prints `git-optional-locks` on stderr unless the caller's `GIT_OPTIONAL_LOCKS` is exactly `0`; unset, `1` and `false` print it; a disk project omits it. Nonliteral Git ownership destinations refuse as `spec`. A share source that is not a plain directory, and a destination nested in a share that is not reached through plain directories, refuse as `spec`; corrected trees run. Guest traversal of outward symlinks cannot access an unmounted host fixture. Confinement is by path: A share file hard-linked to a file outside the share, Git objects from a local clone included, can be changed in place through the share. |
| 49 | The Rex relay ends with its box generation and never reaches another box; `start` creates it only while Remote Control is off | macOS only. A start in a Rex pane on an image with `rex` prints the relay address, and a host `rex -S` lists and creates guest sessions through it. A second start reuses the socket. With a live owner, a connection held across `stop` ends, the socket is gone and a new connection is refused. After SIGKILL of the owner a held connection ends, no dial exec remains and connections are refused; a following `stop` leaves no socket, and a following start serves only the new generation's server. A start without the relay keeps a live owner's socket. Two concurrent starts share one server and socket. A `stop` during a first start's or a running owner's setup leaves no relay and prints no address for the stopped generation. A host client killed mid-stream leaves no dial exec. After the guest server is killed, the next start relays again. Without `REX_SESSION`, without terminals or without `rex` in the image, start prints nothing new and binds no socket. With Remote Control on, start prints `rex-remote-control` and binds no socket. |
| 50 | A profile with a base applies the base beneath it and is outdated when the base changes | A four-link user chain (a profile on `adam` on `documents` on `default`, the top with a one-line Containerfile) builds root first; each image's `dev.pinfold.base` is the id of the one beneath it. Its box has every link's tools, the base seeds, a seed and a share file the top replaces by path, and the share files of every link; `config --json` reports the top's `allow` over adam's. After a change to the root's recipe, a launch names `image-outdated` for every image above it; after the rebuild, none. On Linux, a base's `latest` moved by another build during the build leaves it on the recorded identity; that identity removed ends it with `image-base-changed`. A cycle, a self-reference and a missing base refuse as `profile` in `box up` and in `config`; correcting the base runs. A profile without `base` keeps its own share directory mounted live. `full` is an unknown profile. |

## Code

Rust. Each Linux build is a static musl binary.

Every process-interface operation is a `core` function that returns data;
`cli.rs` parses arguments and serializes results. The one exception is
`clean`'s measurement plan, which needs the pi layer's project state and
lives in `cli.rs`.

| Target | Role |
|---|---|
| `aarch64-apple-darwin` | macOS CLI |
| `aarch64-unknown-linux-musl` | init on Macs |
| `x86_64-unknown-linux-musl` | Linux x64 CLI and init |

Dependencies: `tokio`, `httparse`, `serde`, `serde_json`, `sha2`, `toml`, `nix`
(with `socket` for vsock, `sched` and `mount` for the VMM jail; Landlock
through nix's `libc` syscalls),
`ring` for host builder-file and image-store SHA-256, and the TLS client for
injecting routes: `rustls` (ring) with `rustls-native-certs` for the host's roots.
SNI comes from rustls's ClientHello acceptor. The Public Suffix List is
data, `crates/pinfold/public_suffix_list.dat`, embedded with `include_str!`.

- Platform dirs are the literal XDG-style paths on both OSes:
  `~/.config/pinfold`, `~/.local/state/pinfold` and `~/.cache/pinfold`,
  with `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` honored.

## Open questions

- `198.18/15` blocking breaks fake-IP DNS proxies (Surge, Clash).
  Configurable, or documented.
- Linux arm64 returns once a KVM host can run its suite; GitHub's arm64
  runners have none.
- Where herdr reads `HERDR_AGENT`.
- Freed blocks inside a disk: whether they return to the host, or a
  disk's allocation only grows.
