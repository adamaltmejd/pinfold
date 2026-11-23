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
Core reads only `base` from a profile's `pinfold.toml`; the pi layer reads
the rest when it writes a box spec. Profiles hold no code except pi packages.

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
| Profiles and configuration | Directory admission excludes protected configuration and profile paths from projects, their read-only folders and shares (Configuration, Mounts). It does not exclude allowed hardlinks from shared files to outside configuration or profile files. |

Not protected:
- A project's read-only folder (Configuration) exposes all its readable
  contents to the box, which can send them to any allowed host. A folder
  whose path lies inside a directory another box can write, or under a
  home directory reached through such a directory, is chosen by that box.
  A bind mount of part of a project or a protected path is not recognized
  as inside it.
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
- The Git remote's guarantee ends at refs under `refs/remotes/box/*` and
  new tags under `refs/tags/*`, and Git writes them:
  - A refspec on the command line that maps a ref to another name
    (`git fetch box main:other`, a branch to a tag, a tag to another tag)
    and `git pull box` write, and with prune delete, the user's own
    branches or tags, and `git pull` checks out box content.
  - With `--prune` or `fetch.prune`, the box deletes `box/*` branches by
    deleting its own.
  - A fetched box tag is the user's tag: `push.followTags` may publish it
    to another remote. One named like a host branch makes that short name
    ambiguous, so `git push box main` fails until it is deleted.
  - The fetch lists host tags as they stood when the helper read them: a
    host tag created or moved during a pruning or tag-forcing fetch may be
    deleted or reset.
  - A box branch named `HEAD` lands as `refs/remotes/box/HEAD`, so `box`
    names a commit the box chose.
  - A fetch may update some `box/*` refs and fail others, such as `x/y`
    beside an existing `box/x`. `--atomic` moves none, but leaves the
    checked objects in the repository.
  - Git's own fetch behaviour applies: it writes `FETCH_HEAD`, may run
    auto maintenance, and may fetch submodule commits the box's history
    names from the submodules' own remotes.
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
- pinfold trusts the first `op` on the launching user's `PATH`, run with
  that user's environment, as it trusts that environment for `from`. A
  profile or project file can name any 1Password item the user's `op`
  session can read, and pinfold puts its value where the file says: in
  the box's environment or in a route's header to the route's origin. A
  box spec a caller hands `box up` can equally name any such reference.

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
(e2fsprogs), and a statically linked pinfold executable: it is also the
guest init, so one with a `PT_INTERP` program header is refused with the
token `static` before any disk is prepared. Without user
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
   mount, before deeper attachments. Then create missing generated-mount
   parents as the box user, without changing existing ownership, and only
   then attach generated configuration. Preparation never marks a disk clean. On Linux a
   failed start may retain a reused disk’s unclean mark before BOOT even when
   no disk was written. An ordinary
   directory population already created is retained; a symlink or non-directory
   anywhere along the generated path fails startup.
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
{"event":"built","image":NAME,"ref":"localhost/pinfold/image-<NAME>:<build>","latest":"localhost/pinfold/image-<NAME>:latest","id":ID,"labels":{…}}
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
monotonic for the box's life; the caller keeps its own baseline. A
command the guest kernel OOM-kills exits 137 itself; on Linux `oom_kills`
then rises. On an
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
On Linux the box boots the image id `up` resolved, under a lease taken at
resolution, so a moved tag cannot change what runs. On macOS the runtime is
given the reference; when it reports a different image than the one
resolved, `up` fails with `image-changed` in `detail` before `ready`. A
project start whose tag moves between the client's and the owner's
configuration check gets `config-changed` naming `image.id`; one whose tag
moves after the owner's check gets `owner-unavailable`, on either platform.
No test covers these checks: each window lies inside one start, a timing
race.

`down` of an absent, empty or flag-like name other than `--help` and `-h`,
which print help, exits 0, prints nothing and
leaves any live box alone. `down` uses a host-only control socket and addresses
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
  `{ "from": "NAME" }` reads the caller's value, and `{ "op":
  "op://VAULT/ITEM/FIELD" }` reads it from 1Password (below). A `from`
  name, here, in an injecting header or in a login route, that is empty
  or holds `=` or NUL is never read and is refused as `spec`, naming only
  its key. Guest
  values reach the
  guest after boot, over pinfold's own channel to init (Always applied);
  pinfold passes none to the runtime as environment, so they cannot
  configure the host runtime client. A `from` value never reaches argv,
  the runtime's records or a file pinfold writes. A literal `HOME` is also
  the path that places generated mounts (Pi login), so it appears in
  their mount arguments, Apple's container configuration and the launch
  record, and a refusal of such a mount may name it;
  multiline values remain intact. The runtime executable is found on the
  host's `PATH`, never the spec's.
- `op` takes the place of `from` wherever `from` names a secret: an env
  entry, an injecting header, a claude or pi login route. A reference
  starts with `op://`, holds no control character (below 0x20, or
  0x7f) and is at most 1024 bytes. `from` and
  `op` together, or neither, are refused as `spec`, naming only the key,
  and a configuration file holding such an env entry or injecting header
  fails to load; so is a
  malformed reference, and an explicit null for either key. Only the process that starts the box
  reads a reference, and only after every check that needs no value:
  `box up` after the spec (an env or header `from` included), profile, mount preparation,
  runtime, image and home-seed checks, just before it resolves login
  routes and claims the name; a starting project client after every
  launch check, just before it spawns the owner. It runs the first `op`
  on the host's `PATH`, with its own environment, as `op read
  --no-newline REFERENCE`, once per distinct reference, stdin null,
  stderr discarded, in its own process group; a cancelled launch starts
  no further `op`. The read has a 30-second deadline and stdout is
  bounded to 64 KiB; on the deadline, on cancellation or on a signal
  pinfold handles, the group is killed and `op` reaped. A SIGKILL of the
  launching process leaves `op` to end on its own. A failed read refuses the start as an unset `from` variable
  does (`spec` for an env entry or injecting header, `login` for a login
  route), naming the key, the reference and the cause: `op` not on PATH,
  its exit status or signal, the deadline, an empty value, more than 64
  KiB, or a value that is not UTF-8 or holds NUL. No message holds any of
  `op`'s output. A reference is not secret: it appears where a `from`
  name does. Its value follows the `from` value rules, but never enters
  an environment on the host.
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
  read-only after every caller attachment. On Apple it attaches at its
  host directory's path, and a share or writable disk whose destination
  contains or lies within that directory, or with egress
  `/var/host-services`, refuses as `spec`.
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

The record never holds an env value, a header name,
value or prefix, a login token, argv or command output. `env` lists the
names of the variables the box got, never their values: the declared ones,
pinfold's own default `PINFOLD_ALLOW` and, for a project box,
`PINFOLD_PROJECT` (Process interface).
Always-applied names and Apple-generated Git configuration names are
omitted unless the caller declared them. A route's `to` is its `host:port` or origin.
The record is what this run applied at start; a later change to config leaves it as it is.

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
  as the host uid and sets `no_new_privs` on itself first; failing to set
  it fails startup. The guest kernel boots with
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
host share server enforcing it. The uid, capability, `no_new_privs`,
read-only root and nested user namespace controls above prevent ordinary
box processes from removing or replacing the read-only mount. A guest-kernel compromise is outside this protection.
Every process that looks at a share runs as the host uid. On Linux box work
cannot list the exec service's `/proc/<pid>/fd`.

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

The VMM's standard descriptors are `/dev/null`.

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
  guest terminal ioctl failure fails execution. Later host resizes reach
  the guest terminal while the command runs.
- Each project client owns its terminal and command transport.
  Apple execution, project and public, carries command frames over the
  native client's private pipes and creates its PTY in the guest. Only
  pinfold changes the caller's terminal. No native Apple client holds a
  caller-terminal descriptor. Framed execution keeps separate stdout and stderr channels.
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
  pipe reader so its writer observes SIGPIPE. An observed command status is kept.
  An observed exec status survives later relay failure or cancellation.
  After the command exits, remaining output drains to EOF or a one-second
  deadline; a descendant holding an endpoint cannot retain the client.
  Both platforms restore only termios this execution changed. Restoration
  checks that its process group owns the foreground terminal before writing.
  A non-raw execution leaves later terminal settings alone. Handled
  cancellation restores owned changes and prevents late raw setup. Apple
  never changes caller file status flags. A real terminal exec preserves
  the caller's input file status flags, blocking or nonblocking. Private stdio copiers carry no owner
  authority, exit on client death, and are killed and reaped on cancellation.
  A restoration failure
  fails an otherwise successful command; an existing command failure is
  preserved. This does not cover process-killing signals.
- When a project client departs, its unprivileged guest helper sends TERM
  to that command's process group, then KILL two seconds later if it remains.
  A terminal departure first closes its PTY master, hanging up the guest terminal.
  Client signals, Leave after a signal, lease EOF and control connection loss
  are departures. A crossing's box command is a project command whose
  client is the crossing process itself, so it ends however the crossing
  ends: `git-remote-pinfold` treats Git closing the helper's input as
  SIGHUP, from before it waits for the project through box startup and
  the crossing, so a killed Git ends it. The guest helper watches connection loss independently of command
  I/O. Helper setup or relay failure also ends the command group and reaps its
  leader.
  The group comes from the helper's own child, never workload data.
  Other sessions, the helper and relay are untouched. Normal command completion
  leaves background children running, with output delivered for up to one second
  after the command exits. For `box exec` and project commands on both
  platforms, the caller gets the command's status when it exits, once its
  output ends or that second has passed. pinfold does not signal a process
  that survives the command's exit and still holds its output or terminal:
  its later output is discarded, and its guest relay ends when the last such
  descriptor closes. A process that leaves the command's process group,
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
  record. A completed exec
  result is checked before a ready shutdown or signal. Terminal-restoration
  failure preserves an observed nonzero command status and changes observed
  success to 125.
- Every process started in a box through `exec` starts with `oom_score_adj`
  1000. On Linux, unprivileged work and its descendants may lower it to
  -999, but cannot obtain score-based OOM immunity through either
  `oom_score_adj` or the legacy `oom_adj` interface: -1000 and legacy -17
  fail with `EACCES` and leave the score unchanged, while legacy 15
  succeeds. The exec service
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
pins that claim's generation state. Claims pin disks as Disks, Attachment,
states; recovery of other verified-dead claims continues.

## Egress proxy

In the pinfold process, one per box, listening only on the box's socket. No
network listener, no token: the socket identifies the box.

- **CONNECT:** port 443 only, to an allowlisted host (exact name, or
  `.suffix` for the name and its subdomains). The ClientHello SNI must equal
  the CONNECT host.
- **Plain HTTP:** port 80 only, one request per connection, Content-Length
  framing only. Ambiguous framing and bare LF get 400, as does any head the
  parser rejects, folded headers included; a CONNECT head is parsed the
  same way. `Expect` is never forwarded. On an HTTP/1.1 request, its
  fields form one comma-separated list, empty members ignored; when it
  has a member and each,
  trimmed of SP and HTAB and compared ignoring ASCII case, is
  `100-continue`, the proxy itself answers `100 Continue` once the request has passed every check above and below and
  the upstream is connected; any other `Expect` gets 417, reason
  `expectation unsupported`, before any other check of a parsed request.
  HTTP/1.0 `Expect` is ignored.
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
  sets each named header to `prefix` plus `$VAR`, replacing any the box
  sent under that name. `$VAR` is read once per start, from the
  environment of the process that starts the box: `box up`'s own, or a
  project box's starting client (Configuration); a missing variable
  refuses the start. A header may name `"op": REFERENCE` instead of
  `from`, read by that process (Process interface). An `https` target is
  resolved and checked like an allowlisted host; an `http` target is a host
  service. The value never reaches argv, the box, the spec or the log. The
  box can use the credential but not read it, unless `to` echoes request
  headers back. Header names that frame the request (`Host`,
  `Content-Length`, `Expect`, hop-by-hop) are refused.
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
    Bearer`. `"op": REFERENCE` may replace `from`, here and for pi.
    Claude's managed settings set `ANTHROPIC_BASE_URL=http://<route>`,
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
    `requires_openai_auth = false`.
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
    P, `from` or `op` with `openai-codex`, or two routes for one pi provider
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
    Generated file contents never enter the home disk.
  - `up` refuses a login it cannot use (no token, an empty or unset
    `from` variable, a failed `op` read, or not a usable JWT) as `login`,
    naming the agent.
- **Limits:** a connection cap and a header timeout. A tunnel is idle, and
  closed, only when neither direction has carried bytes for 5 minutes;
  its log line names `idle timeout`.
  A write blocked for 5 minutes closes the connection even when the other
  direction has activity; its line names `write timeout` unless either
  direction ended idle, which names `idle timeout`. A fatal relay error
  closes both directions.
- **Log:** one JSON line per decision in the run's egress log at
  `~/.local/state/pinfold/runs/<run_id>/egress.jsonl`, so a reused name
  starts an empty log. It names the decision's UTC
  time (RFC 3339 to the second), the host, the decision and its reason. A
  route request's line also names the method, the request path without its
  query, and the upstream's status (null when no status line arrived). No header value is ever written.
  A refusal's reason is one of `not allowlisted`, `ip literal`, `route`,
  `port not allowed`, `sni missing`, `sni mismatch`, `malformed clienthello`,
  `malformed request`, `ambiguous framing`, `expectation unsupported`,
  `header timeout`, `connection cap`, `resolve failed`, `login lapsed`, `login credential query`,
  or a refused address's class: `loopback`, `unspecified`, `private`,
  `link-local`, `cgnat`, `benchmark`, `ietf-protocol`, `multicast`,
  `reserved`, `site-local`, `nat64 local-use`.
  Each box's log is capped at 128 MiB. A complete encoded JSON line,
  including its newline, that would exceed the cap writes nothing. A line
  that fits exactly is appended. There is no rotation or truncation.
  An append failure or cap exhaustion stops the box with `audit-log`.
  Either can prevent recording a request already forwarded to its upstream;
  teardown follows the owner notification.
- **Refusal responses:** every response the proxy sends for a refusal,
  plain HTTP or CONNECT, at any status, names the reason its log line
  records, the same string. It carries `Proxy-Status: pinfold;
  error=TOKEN; details="REASON"` (RFC 9209), where TOKEN is
  `http_request_denied` for `not allowlisted`, `route`, `port not
  allowed`, `login lapsed` and `login credential query`,
  `http_request_error` for `malformed request`, `ambiguous framing` and
  `expectation unsupported`, `dns_error` for `resolve failed`,
  `proxy_internal_response` for `connection cap`, and
  `destination_ip_prohibited` for `ip literal` and a refused address's
  class. The header is always sent. A `text/plain` body is sent only
  when the request parsed (a CONNECT whose target splits at its last `:`
  into a non-empty host and a port, valid or not, or a plain HTTP head
  that passed the checks above) and its method is not `HEAD`. It says
  the pinfold sandbox refused the request, gives the reason, and says
  allowed hosts and routes are set on the host. A refused parsed `HEAD`
  has no body and no `Content-Length`. Every refusal before the request
  parsed, including `connection cap` and a 400 from parsing the head or
  the CONNECT target, has `Content-Length: 0` and no body. Neither holds
  anything from the request. Refusals that
  get no response (`header timeout`, the ClientHello checks), the empty
  502 for an upstream that cannot be reached, and responses relayed from
  an upstream are unchanged.

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
  `pinfold -- CMD`, and `pinfold start` right after its first-run setup,
  offer to build a missing managed image. Confirmation
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
  successfully rebuilt images beneath it published. Stderr names each
  image as its build starts; those named before the failed one are the
  ones left published.
- After each completed Apple build, pinfold attempts to prune the shared
  builder's unused cache to 16 GB, then return unused builder blocks;
  failure only warns and preserves the build result. It never stops the
  builder. An Apple build holds a host lock shared, which `clean` takes
  exclusive to stop the builder.
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
daemon.

Built-in images use `debian:trixie-slim` and strip setuid/setgid bits.
`default` names uids 501 and 1000 `pinfold`, home `/home/pinfold`, where
its base image leaves those uids free.

| Profile | Tools |
|---|---|
| `default` | shell, ca-certificates, git, curl, ripgrep, fd-find as `fd`, jq, less, procps, standalone Pi, native Claude, Codex and its code-mode host |
| `documents` | default tools plus Bun, AnyDoc and Poppler |
| `adam` | documents tools plus gh, fff (pi extension) and ponytail |

`profile/<name>/Containerfile` is each built-in's recipe. `documents`
has `base = "default"` and `adam` has `base = "documents"`; each recipe
`FROM`s its base's stable ref and adds only its own tools.
Every built-in recipe ends with the step that strips setuid/setgid bits,
so each link strips what it added. A user recipe's steps are its own.
Infrastructure and tool assets use pinned `ADD --checksum` inputs for the
selected architecture. Only `default`'s recipe resolves and installs
agents (Images); the profiles above it inherit them from its image.
A per-build argument is declared only by the resolver stage; no other
step depends on it. Installation uses publisher integrity metadata where supplied. Codex and its code-mode
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
host helper, `codex-app-server`: its version and assets per host `os-arch`
(`darwin-arm64`, `linux-x64`). The verified installer puts it in
`~/.cache/pinfold/artifacts/codex/<version>/host-<os-arch>/` on the first
`up` with a codex or pi `openai-codex` login route. A missing recorded file reinstalls it. A checksum mismatch fails the login.

`scripts/bump-pins.py` moves the host helper, infrastructure, image-tool,
Rust and Cargo pins only with a pinfold release. It moves the Public Suffix
List only with `--suffix-list`, which a manual release passes and the
nightly does not; that run prints the allow entries newly refused or no
longer refused, and the release notes name them under "Caller changes".
Versions only move up to stable releases and are never rehashed at the same
version.
The helper takes the latest stable release once its assets exist; other image
tools wait seven full days after their last required asset was published.
A missing publication time makes them ineligible. Cargo dependencies take
the latest stable, non-yanked version. Publisher integrity failure aborts
before any pin file is written.

On Linux, `crates/pinfold/vmm.toml` pins Firecracker and its guest kernel
for `linux-x64`, each checked against its pinned digest. The guest kernel
is pinfold's own build of a pinned kernel.org stable release, verified
against kernel.org's signature, without vsock loopback and with its config at `/proc/config.gz`; `guest-kernel/`
builds it. Both use the verified installer, into
`~/.cache/pinfold/artifacts/vmm/<version>/<os-arch>/`, where `<version>`
names Firecracker's and the kernel's, on the first `up`.

`builder-root/` builds the Linux x64 bootstrap (see its README); the
tar.gz holds only the 512 MiB read-only ext4 root.
`crates/pinfold/builder.toml` embeds the published archive URL and reviewed
archive, raw-root and recipe identities. Pinfold acquires the archive over HTTPS
into install staging. The offline archive cache
is `~/.cache/pinfold/artifacts/builder/<archive-sha256>/`; when present, its
archive is copied into that staging area instead. Either way the archive
is bounded to its pinned size, below the shared 512 MiB transfer cap. Acquisition,
verification and installation share one cancellable 300-second deadline.
Installation verifies the staged archive's pinned size and SHA-256 before
streaming only `root.ext4` into a fixed 0600 file and atomically publishing
the complete `linux-x64/` install. The raw-root size and SHA-256 are verified
before publication and on every use; a mismatch fails with
`builder-integrity`. Embedded digests remain authority if public bytes
change; the sidecar manifest supplies none. `doctor --artifacts --json` reports
the builder URL, archive pin/size, recipe SHA-256 and verified cache state.

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
  `profile/<name>/`, laid out as above with only that profile's own
  files; a file's path under `home/` or `share/` is its path in the
  guest. A user profile with the same name replaces the whole
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
- `pinfold profile` (or `profile list`) lists every profile: its base, and
  its directory, editable, or that it is built in. In a configured project
  it marks the selected one.
- A profile whose chain has no recipe needs an explicit image or, for project commands, a project recipe.
  Without either, core refuses as `profile`, with `image-source-missing`.
- `home/` needs `$HOME` on a writable disk; seeds (Terms) are copied in at
  each start. Changing a profile does not reset disks or overwrite saved seeds.
- pinfold does not manage pi settings: `pi install` in a box changes that
  project home only.
- Built-ins package all three agents on PATH. Missing-only home seeds
  supply Pi's trust/settings defaults, Claude permission/sandbox defaults
  and Codex approval/sandbox/update-check defaults. In the `default`
  image, `claude` on PATH is a launcher that, in a project box, first sets
  Claude's onboarding, auto-mode nudge and `$PINFOLD_PROJECT` trust flags
  in `~/.claude.json`, so no first-run screen undoes the seeded defaults. Native home instructions
  are `.pi/agent/APPEND_SYSTEM.md`, `.claude/CLAUDE.md` and `.codex/AGENTS.md`.
  An existing instruction file blocks its missing-only seed, including an
  earlier seed; changing profiles does not update it. Instructions are
  guidance and enforce nothing. Supported updater switches are recipe environment defaults. Route-generated
  system files contain authentication configuration only.

pi's two config levels both load in every run: the user level (pi's agent
dir under `$HOME`, seeded from `home/`, with packages from `share/`) and
the project's own `.pi/` and `.agents/skills`.

Every built-in seeds only `/opt/pinfold/profile/pi` as its pi package and
sets `defaultProjectTrust: "always"`. It adds
pi's built-in `codemode` tool to the default tools. Every built-in includes the operating-context extension, which
writes into pi's system prompt at every start the allowlist from
`PINFOLD_ALLOW` and that a 403 carrying `Proxy-Status: pinfold` is the
sandbox's final refusal, which `curl -v` shows. pi therefore gets
the refusal line even when an existing instruction file blocks the seed.
The seeded instructions carry the same refusal line for Claude and
Codex, and where work goes in a share and a disk project. Only `default`
carries home seeds;
the others inherit them. Documents adds `read-documents`, which adam
inherits. Adam's own `pi/package.json` replaces default's and also
registers fff and ponytail.

Default and documents set no config besides `base`. Adam's `allow` is the default list
plus `github.com` and `api.github.com`.

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
  An Apple preparation error is cleaned up as a failed `up` is (Process
  interface), its helper container in the runtime's place. Preparation
  stderr retains only its latest 64 KiB in memory and is escaped when reported.
- **Clean:** a writable attachment records the disk unclean. A stop
  pinfold requests of a running box (step 8) syncs and unmounts every
  disk, and pinfold records each clean once the runtime confirms it. A
  disk whose writable box ended on its own, or whose owner died, stays
  unclean: a read-only attachment is refused as `disk-unclean` until a
  writable one, which recovers the disk, ends with `down`.

### Crossing

A disk's `from` copies host files in once, when pinfold creates the
disk; the pi layer's disks name none. Otherwise data crosses only as git
branches and tags, through a disk project's Git remote (Git), against a live box and run in it as the box user, as a project command in the project root. Neither
direction touches a working tree.

- **Push** bundles the host refs Git names and fetches them in the box,
  branches into the box repository's `refs/remotes/host/*` and tags into
  its `refs/tags/*`, in one transaction. The box merges or rebases them
  itself. Host data is trusted in this direction. When an unforced tag
  already exists in the box at another object, the box changes nothing
  and exits with a dedicated code, which fails every ref of the push with
  that reason; `--force` or `+` replaces the tag. Git, planning from the
  box's listing (Git), usually rejects such a tag before. When
  the box path has no `.git`, the push first runs `git init` there, with
  the host repository's object format, and checks nothing out; HEAD
  names Git's default branch, unborn, and `git switch -c BRANCH
  host/BRANCH` in the box checks a branch out. Two such pushes may both
  run it: a second `git init` on an existing repository leaves its refs
  and objects, and HEAD stays unborn. A nonzero box exit fails the push.
  The push reads nothing from the box but its exit status, and prints
  none of the box's text; Git's push plan reads the checked listing
  (Git).
- **Fetch** has the box bundle its repository's branches and tags, less
  what the host's `refs/remotes/NAME/*` already hold, where NAME is the
  remote; an annotated tag's object is bundled every time. The box first
  lists its branches and its tags that peel to a commit, whose names are
  printable ASCII and that are not symbolic refs: lines of at most
  4096 bytes, each `OID refs/heads/BRANCH` or `OID refs/tags/TAG` with a
  full object id of the host repository's width (40 hex digits for
  SHA-1, 64 for SHA-256), then an empty line. More than 4096 listed refs,
  or one listed twice, fail the fetch. Every listed branch name must pass
  `git check-ref-format` under `refs/remotes/NAME/` before git sees it; a
  new tag name that fails it as itself is left out. The bundle's heads
  are branches and tags the listing names, each at its listed object; any
  other head fails. Symbolic tags and other refs stay in the box; a
  symbolic branch is listed at the commit it resolves to. A nested annotated tag is listed only where the box's
  Git peels `%(*objecttype)` fully (2.43 does not). A bundle past 4 GiB is refused as `limit`, and the
  box's command is killed. On the host, pinfold runs `git bundle verify`,
  `git index-pack --fsck-objects` of the bundle's pack, a connectivity
  check of every object that pack carries, a partial clone's promised
  objects excepted,
  and `git fetch` of the bundle, all in a
  quarantine object directory inside the repository's, where a partial
  clone's lazy fetches land too; a filtered bundle fails. That fetch names no destination and
  runs with `--no-prune --no-prune-tags --no-tags`, so it writes and
  deletes no ref whatever the user's configuration. Every listed branch
  names a commit, never another object type: a bundle head the box's
  pack carries itself (not a base `--fix-thin` added from the host),
  which the quarantined fetch checked connected, or one
  `refs/remotes/NAME/*` already reaches. A listed tag names such a
  commit, or an annotated tag object that is a bundle head the box's pack
  carries and that peels on the host, read through the quarantine, to
  such a commit. An object the host holds otherwise is never the box's to
  name. A bundle head that fails fails the fetch; a tag outside the
  bundle that fails is left out. A fetch never changes or deletes a host
  tag, but through a command-line refspec that maps a ref to another tag
  name (threat model); a host tag that is itself a symbolic ref to a
  `refs/remotes/NAME/*` ref follows that ref, and a host tag listed back
  under a branch's name makes the short name in `git fetch box NAME`
  select the tag. A host tag whose name is not UTF-8 could not be
  listed back as itself, so the fetch refuses as `spec`. A box tag whose
  name matches a host tag's ignoring ASCII case is left out, as loose refs
  are files on a file system that may fold case, and so is a non-ASCII
  one. While any host tag's name is non-ASCII, no box tag is listed, as
  Unicode normalization could match an ASCII name to it. Every host tag
  is listed back at its own id, so
  neither `--force`, a `+refs/tags/*` refspec nor pruning
  (`fetch.pruneTags`, `--prune-tags`, `--prune --tags`) finds a host tag
  to move or delete. A left-out tag's objects may still enter the object
  store, unreferenced, with the rest of the bundle. Each new annotated tag is listed with its peeled commit, so
  Git follows tags as from any remote. A bundle carries
  no hook or config, and nothing is checked out. Only refs that pass
  every check reach Git, and the quarantine's packs move into the
  repository only when Git asks for their objects: each index after its
  pack, a partial move undone, and a quarantine another fetch swept away
  fails. Moves into one object store run one at a time, under a lock on
  its `pack` directory. A rejected bundle leaves the repository's objects and refs as
  they were. pinfold removes the quarantine and its bundle file as the
  helper exits. Creating a quarantine removes the repository's
  quarantines, and creating a bundle file removes the state root's bundle
  files, whose name is a canonical positive pid that no process has or
  that is the creating process's own; any other name stays, and a reused
  pid keeps its leftover until that process ends. Fetch has no deadline or
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
any existing DEST. An existing DEST, a symlink included, is refused with
exit 1 and `destination-exists` on stderr. The destination root is 700.
Export does not repair an unclean disk or change Pi's branch crossing.

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
  cases below, and not `--version`, `--help`, `doctor`, `config`, `profile`, `profile list`, `update`, `box list`, `box down`, `box inspect`, `box events`, `box prune`, `--`, `stop`, the private owner entrypoint or `init`): prune boxes whose owner
  is gone (nothing owns its generation's lifetime lock), leftover sockets,
  artifact `<name>/<version>` directories no pin names (`init/` aside),
  builds beyond each source's newest two (the after-build rule, applied
  even when no build follows), and inactive runs' records (below).
  `box up` runs this pass after admission, before startup preparation; an
  admission refusal runs no pass. Project commands defer it until their
  session lease has ended.
  `start` runs it after keep is committed, without a client lease.
  The Git remote helper defers it until the running owner’s mount decision
  and the configuration's loading and protected-path check. A refusal
  before then, a share refusal included, runs no pass. Signal handlers are installed before this deferred maintenance.

A run is inactive when its generation owns no lifetime lock. An inactive
run's directory goes when its newest file is older than 14 days; then,
while inactive runs together exceed 128 MiB, the oldest goes first, so
the quota can remove an egress log before 14 days. A live run's records
are never removed or truncated. Its egress log has the Egress proxy's size limit.

`pinfold image rm NAME` retires a caller image name: it removes every tag
of NAME, whatever its age, and no other name's.

`pinfold clean` lists sizes, then removes:
- everything automatic, now
- the unused build cache (Apple: prune BuildKit records, return their
  blocks to the host, then stop the idle builder, which releases build
  outputs the runtime still holds; a later build restarts it with its
  cache and root filesystem. The builder is idle when no pinfold build
  holds its lock and BuildKit lists no build without a completion time.
  Otherwise, or when the builder does not answer within a deadline,
  clean leaves it running and says so; that warning does not change the
  exit code. Pinfold's own builds are always excluded by the lock. Builds
  started outside pinfold are seen only through the builder's own build
  records, so one it does not yet report (a client connected but not
  submitted, a builder configured to keep no history) can be failed by
  the stop. Linux: remove
  only an exclusively claimed idle owned builder cache)
- with `--idle AGE`, state and disks of projects not run for that long,
  except a project's with a live box. Never automatic: a project disk
  holds unfetched work, and a home disk sessions and logins.

A project's configuration is the user's; nothing removes it. A project
whose checkout is gone or moved keeps its state and disks;
`clean` lists them, and they go only with `--idle AGE` or `disk rm`.
Other disks go only by `disk rm`; a project home's caches are inside its
disk and not cleaned. The listing counts the disks and names both ways
to remove them.

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
through teardown, including while kept without clients, so cleanup skips
its project.

## pi layer

- **Command:** `pinfold -- CMD` runs the complete literal argv in the image.
  The executable word must be nonempty. No shell or implicit agent is added.
- **State:** a project has host state under
  `~/.local/state/pinfold/projects/<name>-<hash>/` and a persistent
  `pi-<name>-<hash>-home` disk at `/home/pinfold`. A disk project also has
  `pi-<name>-<hash>-project`. The hash is of the canonical project root
  path. Agent settings, sessions and caches sit at their default paths in
  the home disk. The cosmetic name is at most 32 ASCII bytes.
- **Project mount:** a share appears at the canonical host project path,
  exposes host files directly and creates no project disk. A disk appears
  at `/project`, is created empty and is reused across owners; code enters
  it only through the box or an explicit push (Git). Git metadata directories and operational discovery errors never
  become non-Git projects. Independent nested repositories retain their own
  project discovery. The project's read-only folders (Configuration)
  appear at their guest paths.
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
  stop (Process supervision), even while a keep request or crossing lease
  retains the box. Ending one session does not end another.
  Two share sessions use one generation, one share and one home disk.
  With no session, crossing lease or keep request, the owner closes admission
  and tears down the box. Bootstrap death or a final departure racing a
  join, keep or stop never admits two writable generations.
- **Startup:** after bootstrap validation, claim a fresh box name and publish
  control before runtime checks and image leasing. Existing name records,
  state or shutdown endpoints refuse. These checks run as retained cancellable
  work; catalog and image-lease locks wait nonblocking with cancellation.
  Public box-up checks and orphan recovery keep their existing order.
- **Keep/stop:** `pinfold start` keeps the project box past its last session.
  Repeated matching starts succeed. For a share, start commits keep without
  executing a command. On success `start` prints one stderr line,
  `pinfold start: started` or `pinfold start: already running`, before any
  Rex lines. `pinfold stop` targets the observed
  generation, works without current configuration, and succeeds only after
  teardown or confirmed absence, printing on stderr `pinfold stop: stopped`
  or `pinfold stop: no box running`. For a share, it revokes that box’s filesystem
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
  unix:///home/pinfold/.local/state/rex/run/server.sock` in the box and waits
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
  relay up again. `start` prints on stderr two lines: `pinfold start: rex:
  add this address in the Rex app under Add Remote Host…`, then
  `unix://PATH` alone. Remote Control
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
  Concurrent starts share one server exec and one socket.
  Owner death ends every connection and every dial exec, and leaves a
  socket that refuses connections; the guest server's exec ends with the
  orphan box. Only an
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
  `config-changed` (`env.values`). An `op` source compares by its
  reference (`env.names`, `routes.declarations`), never its value. Absent
  and empty differ. Command
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
  never reported. An owner whose box start is refused as `spec` or
  `login` passes the reason token and detail of `up`'s `refused` line to
  the client, which prints both and exits 125; any other refusal's detail
  can carry a tool's output, so it passes the token alone, which the
  client prints with a fixed sentence.
  Unavailable ownership refuses as `owner-unavailable`;
  a control deadline expires as `owner-timeout`. Transport failures retain a
  fixed source (`connect`, `inspect` or `reply`) without request contents.
  Peer and recoverable accept errors reject only that connection. Queued
  requests and in-flight parsers receive `Draining` concurrently with teardown.
- **Arguments** pass through unchanged, including empty arguments and
  `--help`. Commands start at the canonical invoking directory's path
  relative to the project root, below the guest project root. Trusted
  init opens the guest project root and walks the relative cwd by directory
  descriptors without following any symlink component, then enters that
  descriptor. An escape, missing directory or any symlink component,
  including a symlink within the project, fails setup. An execute-only
  directory on that walk works. A non-UTF-8 path in
  a project box spec or configuration
  report is refused as `path-not-utf8`, naming its field.
- **Environment:** the core's proxy variables, `PINFOLD_ALLOW`,
  `PINFOLD_PROJECT` (the project's guest path, unless `env` sets it), the
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
  `stopped`. CMD owns stdout; diagnostics use stderr.
  A share has no project disk, disk attachment, unfilled marker or clean/unclean
  state. Its home retains managed-disk claims and recovery rules. Owner discovery requires no project disk; unknown
  runtime liveness preserves ownership and prevents replacement.

### Git

This section applies only to disk projects. Shares have no project
crossing. The Git remote refuses a share as
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

The project disk holds the box's own repository, once the box creates or
clones one or a push creates it (Crossing). Only the host's branches and tags enter
it from the host: no host `.git` content, config, hook, stash, reflog,
or untracked or ignored file, and a submodule only as its gitlink.
The agent commits there,
under the box's hooks and config, none of which reach the host. Built-in
images set git's system identity to `pi <pi@pinfold.invalid>`; a
repository or home identity overrides it.

**The Git remote.** The user adds the host repository's remote `box`
with `git remote add box pinfold::ROOT`, ROOT being the canonical project
root; pinfold never writes host Git configuration. Git runs it
through `git-remote-pinfold`, a link to the pinfold executable on PATH;
invoked by that name, pinfold is the helper. `pinfold update` after an
install, and first-run setup that saves a disk, create that link beside
the resolved executable, relative, only where no file or link exists;
setup prints the `ln -s` command, or names the taken path, when none is
then on PATH, and `doctor` reports a disk project's missing or foreign
one. `git fetch box` is
Crossing's fetch: branches into `refs/remotes/box/*`, and the box's new
tags into `refs/tags/*` by Git's tag rules (followed tags, or every
listed one with `--tags`); no tag it lists changes or deletes a host tag. `git push box
BRANCH` is Crossing's push into the box's `refs/remotes/host/*`, and a
pushed tag lands as the box's tag. The user merges;
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
  with the repository's object format, the checked branches and new
  tags, each new annotated tag's peeled commit, and every host tag at its
  own id, and moves the quarantine's packs in at
  Git's `fetch`; it serves one listing per process;
- pushes each `refs/heads/SRC:refs/heads/BRANCH` Git sends to the box's
  `refs/remotes/host/BRANCH`, and each `refs/tags/SRC:refs/tags/TAG` to
  the box's `refs/tags/TAG`, rejecting other pairings, sources (a stash,
  a commit expression) and destinations, and deletions; a box failure,
  an existing box tag at another object included, rejects every ref of
  that push. It reports each pushed branch as the box's
  `refs/remotes/host/BRANCH`, which host Git 2.29 or later reads, so Git
  leaves `refs/remotes/box/BRANCH` where the last fetch put it;
- at `list for-push`, reads the box's `refs/remotes/host/*` and
  `refs/tags/*` through Fetch's listing checks (line bound, object id width, 4096 refs, prefixes,
  `check-ref-format`); a byte after the empty line fails it, and
  nothing of it is written to disk. pinfold then keeps only entries
  whose id is the id of a host `refs/heads/*` or `refs/tags/*` ref (a
  tag ref's own id), from one `git for-each-ref`; it looks up no
  box-supplied id, and every id Git sees is one a host ref points at.
  It answers Git with the tags only, and Git plans the push from them:
  - every branch looks absent: Git sends a branch the push names,
    explicitly or by the push default, each time as a new reference,
    and a matching push (`:`) sends none; pinfold bundles and pushes
    only a branch whose box `host/NAME` is not at the host's tip, and
    runs no push in the box when none is;
  - an unchanged tag is up to date;
  - a box tag at an object no host ref points at looks absent, so
    `push.followTags` sends the host's tag of that name when Git
    selects it (annotated, on a pushed commit), and the box's tag
    check then fails the whole push unless forced;
  - a box tag of that name at another host tip is listed, so
    `push.followTags` skips it, and Git rejects an explicit push of it
    unless forced.

  pinfold always forces branches into `refs/remotes/host/*`. A
  lying box can only make a push skip or attempt a ref, and the listing
  never feeds a fetch. Any listing failure but a signal lists nothing
  and the push goes ahead;
- speaks Git's protocol on its own descriptors: its standard input is
  `/dev/null` and its standard output is stderr.

A refusal or failure prints `pinfold remote: DETAIL` on stderr, with any
box-controlled text escaped, and the helper exits nonzero; Git reports
it with its own exit code. A crossing refusal names `spec` or `limit`.
Both directions use a running disk owner’s box with a temporary crossing
lease, or load configuration and start a transient box on its project
disk without egress when no owner runs. With no project disk, or one
whose box never came ready (Disks), they fail and start no box. Fetch
from a box path without a repository refuses as `spec`.

**First start.** A new project disk starts empty, Git project or not.
A start runs no host Git beyond admission's verification, pushes nothing
and writes no host Git configuration. Code enters the disk only by an
explicit step: a clone or `git init` in the box, or `git push box
BRANCH`.

## Configuration

A project's configuration is
`~/.config/pinfold/projects/<id>/pinfold.toml` under the XDG rules, where
`<id>` is the project's state id (pi layer, State). The project's image
recipe, when it has one, is `Containerfile` beside it. Nothing in the
project folder configures pinfold. Setup I/O is interactive when stdin,
stdout and stderr are terminals and stdin's is already in canonical mode;
pinfold never changes its mode for a prompt. When the file is missing and
I/O is interactive, `-- CMD` and `start` run one setup. It takes each
setting it saves in turn, the profile and then the project mount; one the
environment decides is stated with its source, not asked.
Setup offers a profile, initially `default`.
A set `PINFOLD_PROFILE` selects the profile without asking, and setup
names it and the variable. On macOS setup then asks for the project mount, `share` (the
default) or `disk`; another answer refuses. The question names the
project root and describes each kind: a disk is isolated, and a new one
starts empty (First start); a retained disk is reused as it is. Linux has only `disk`,
does not ask, and states it once. Setup then prints the settings file's
path and the active lines it will hold, from the template that writes
it, and asks to save. Setup never runs host Git. Before saving, it runs the chosen mount's checks that need
no host Git; a refusal there saves nothing. The disk's Git verification
and the comparison with a live owner happen at the start that follows,
so their refusals leave the saved file. Confirmation saves the profile
and mount as active lines of the commented file `config edit` creates
(below). `-- CMD` continues with the original command and
arguments; `start` starts the box. Both then offer a missing managed image's
build (Images).
Refusal or EOF before confirmation exits 125 without creating the file.
If another caller creates the file while the prompt is open, this launch
refuses and leaves that file in place. Without interactive I/O, `-- CMD`
and `start` refuse as `no-config` and leave stdin untouched. `build`, `config` and
the Git remote always refuse as `no-config`. The diagnostic names the interactive
`pinfold -- pi` and `pinfold -- sh` setup commands.
`doctor` reports it as a finding. An unreadable file
or a dangling symlink fails. `pinfold config edit` checks a project as
below, with the profile a run would select (a profile that cannot be
loaded adds no paths, so the editor still opens), creates the file in one step when it is missing, holding the
project mount below and commented examples of the other keys, says so,
and opens it in `$VISUAL`, else `$EDITOR`, else
`vi`; it exits with the editor's code. On Linux every project command
outside Git, `config edit` included, is refused as `not-git`.

The project mount is required. `config edit` creates
`mounts = [{ host = ".", kind = "share" }]` on macOS and
`mounts = [{ host = ".", kind = "disk" }]` on Linux. macOS accepts either
kind; Linux accepts only `disk`. Further entries are read-only folders,
`{ host = "PATH", guest = "/ABS/PATH" }`, with `kind = "readonly"` the
default and the only kind accepted. `host` is absolute or `~/…`, which
expands under the canonical home; a relative host refuses as `spec`.
`host` and `guest` must be canonical in form: no `.` or `..` component
and no repeated or trailing slash, else `spec`. Each folder becomes a
read-only mount of the project box (Mounts) at its host as written,
which is its startup configuration. At each box start, before its
contents are read, the host must be its own real path, with no link in
it (`spec`, naming the real path), UTF-8 (`path-not-utf8`), not at or
inside the project root by device and inode (`spec`: a box can write
the project, so a link there could choose the folder), and must not
overlap a protected path (below); the box layer then checks it as a
mount host. A link swapped into a folder's path after a start is refused
at the next start; a joining client does not touch folders on the host.
On a macOS share project box preparation resolves the host again, which
leaves a real path unchanged. A destination at, inside or containing
the project's guest root, `/home/pinfold`, the profile's
`/opt/pinfold/profile` or another folder's refuses as `spec` at a
start. Other mount shapes refuse as `spec`. A change applies at the next
box start; a different folder list refuses a joining client as
`config-changed` (`mounts`). `build` and the Git remote attach no
folders; a malformed `mounts` list still refuses as `spec`. On Linux
each start copies each folder into an image in the box's state, taking
time and host disk space in proportion to its size.

A project is refused as `host-path-in-project` when any of the protected
paths listed below resolves inside it, or when a read-only folder of it
is, lies inside or contains one (a folder reached through a link refuses
as `spec` first). This check applies before project
state or configuration is created, whether or not the project is a Git
checkout. A read-only folder is checked at each box start, before a
box or disk is created.

Protected paths are pinfold's configuration, state or cache root, the
project's configuration directory, file or Containerfile, the directory of
the selected profile and of each base in its chain, their `pinfold.toml`,
`Containerfile`, `home` or `share`, the selected profile's share mount,
`artifacts/`, on macOS `artifacts/init` and the supplied init's own
directory below it, and the lock
directory. Resolution follows symlinked ancestors
too, including missing suffixes and a dangling profile share symlink:
box content crosses into the project and must not reach them.
`-- CMD`, `start`, the Git remote, `build` and `config edit` check it, and `doctor`
reports it as a finding. `config edit` skips the read-only folders, so a
refused one can be edited out; `build`, the Git remote and `doctor`
skip them too. Project clients perform admission checks before
automatic maintenance or notices. Other working commands may already have
run the daily maintenance pass and written under the state root before
this check.

Layers: environment > the project's file > the profile's `pinfold.toml` >
each base's `pinfold.toml`, nearest first > built-in defaults. Each key takes the highest layer that sets it, lists
included: a list replaces the ones below it, and `allow = []` allows
nothing. An environment variable that is set, even empty, sets its key.
An override containing non-UTF-8 bytes is refused, naming only its key.
A `PINFOLD_ENV_` variable whose name is not UTF-8 is refused the same
way, as `PINFOLD_ENV_*`, naming no part of its name or value; its value
follows the `from` value rules.
Only the environment and the project's file select `profile`; a profile's
own `profile` value is ignored. `env` is the exception to whole-key
replacement: each layer replaces it name by name, so a name a lower layer
sets cannot be unset above it. A configured `HOME` is ignored:
the project box sets it, as it does the always-applied names, and
`config --json` omits it.

A `from` value is read once per launch, from the environment of the
client whose launch starts a new box; a joining or keeping client reads
its current values only for the admission comparison. A `from` name the
box spec refuses (Process interface) refuses the start before any owner
starts. A missing variable
refuses the start before any owner starts, and so does a missing injecting
or login route variable: the client prints `env "NAME" needs the host
variable VAR, which is not set` or `route "NAME" needs the host variable
VAR, which is not set` on stderr and exits 125. The value travels as a
`PINFOLD_ENV_` value does: in the environment of the detached owner and
its runtime client, and in the admission snapshot, held in memory and
sent over the private project socket. The owner starts the box with the
snapshot's values, and a running owner compares a joining client's
snapshot with its own. An `op` value is read only by the client whose
launch starts a new box, after every other launch check, just before it
spawns the owner, so a refusal before then runs no `op`; a failed
read refuses before any owner starts with `env "NAME" needs the
1Password reference REFERENCE: CAUSE` or the `route` form, and exit 125.
A joining or keeping client never runs `op`, and the owner never does.
The value travels to the owner beside the startup snapshot, never in a
host environment or in any snapshot a joining client sends; the owner refuses
as `session-protocol`, before recording anything, values that are not
exactly the declared references, each once, within the read's bounds; admission
compares an `op` source by its reference alone. `config --json` names
the variable or the reference, never a value; elsewhere it follows the
box spec's `env` rules (Process interface). A literal is configuration, not a secret:
`config --json` shows it; the launch record names it, like every box
variable, without its value. Secrets belong in `from` or `op`.

| Key | Env | Default | Meaning |
|---|---|---|---|
| `profile` | `PINFOLD_PROFILE` | `default` | Profile name |
| `base` | — | — | The profile this one builds on (Profiles); profile file only |
| `image` | — | — | Local image reference, independent of the profile; project overrides profile |
| `allow` | `PINFOLD_ALLOW` | `api.anthropic.com`, `platform.claude.com`, `api.openai.com`, `auth.openai.com`, `chatgpt.com`, `openrouter.ai`, `opencode.ai`, `registry.npmjs.org`, `pi.dev` | Hosts the proxy lets through |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes: `name = "host:port"` for a host service, an injecting route `name = { to = "https://…", headers = { Authorization = { from = "VAR", prefix = "Bearer " } } }`, or a pi login route `name = { login = "pi", provider = "openrouter", from = "OPENROUTER_API_KEY" }` (see Egress proxy) |
| `mounts` | — | — | The project mount, then read-only folders (above); project file only |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| `env` | — | `{}` | The box's variables: `NAME = "literal"`, `NAME = { from = "VAR" }` read from the caller's environment at start, or `NAME = { op = "op://…" }` read from 1Password at start; the box spec's `env` (Process interface) |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box from the caller's value, over `env`'s `NAME` |
| — | `PINFOLD_NO_UPDATE_CHECK` | — | `1` disables interactive update checks and notices (Updates) |

The list keys take comma-separated values in the environment:

- `PINFOLD_ALLOW`: host names; a leading `.` makes a suffix entry.
- `PINFOLD_ROUTES`: `name=host:port` pairs; an injecting or login route needs a file.

An unknown key in the project's file or a profile's `pinfold.toml` is
refused, naming the file and the key. `mounts` in a profile's file and
`base` in the project's file are refused.

`pinfold config` summarizes the project folder, mount, read-only folders, selected profile and
its base chain (with the profile the project's file names when
`PINFOLD_PROFILE` overrides it), the selected profile's directory or that
it is built in, the settings path, and the image and its availability.
It reads configuration without changing state, even when the runtime is
unavailable. `doctor` shows the same profile and settings lines.
`pinfold config --json` prints the effective configuration. `env` maps each
configured box variable to its literal or its `{"from": VAR}`, a `PINFOLD_ENV_<NAME>`
as `{"from": "PINFOLD_ENV_<NAME>"}`; the box's default `PINFOLD_PROJECT` is not listed. Its `project` reports its id, home path
and `mount`, containing `kind`, canonical `host` and `guest` (Project mount).
`project.folders` lists each read-only folder as `{host, guest, kind}`,
`host` as configured with `~/` expanded, `kind` `readonly`.
`project.disks.home` names the persistent home disk. `project.disks.project`
names the disk for a disk project and is null for a share. `config` is the
project’s configuration file. `profile_chain` lists the selected profile,
then each base, as `{name, path}`: the profile's directory, or null for a
built-in. `image_built` reports local presence, not
managed provenance or recipe freshness: true or false when
the runtime image inventory succeeds, and null when it cannot be observed;
`image_error` names that failure. On Linux it reads only the local
catalog, so it needs neither KVM nor a runtime on PATH, and an absent
catalog reports false. `image` is the selected reference; `image_source` is `explicit`,
`project` or `profile`. `containerfile` is null for an explicit image,
the project recipe's path, or `profile` for a profile recipe.

`start` checks local image availability before recording project state, as
`-- CMD` does. Joining does not pull or build an image.

## CLI

```
pinfold -- CMD [args…]           start or join a project session
pinfold start                   keep this project's box
pinfold stop                    stop its observed box
pinfold build [--profile NAME] [--no-cache] [--allow HOST]…   build this project's image, or a profile's; prints the ref
pinfold profile [list]           list profiles: base, and editable directory or built in
pinfold profile new NAME [--from PROFILE]   copy a profile to edit as files
pinfold disk list|rm|export … [--json]   list, remove or export managed disks (Disks)
pinfold clean [--dry-run] [--idle AGE]   reclaim unused disk space; --idle AGE also removes state and disks of projects not run for AGE
pinfold doctor [--artifacts] [--json]   diagnose this host and project, or list artifact pins
pinfold apparmor-profile         the AppArmor profile for this executable (Linux)
pinfold update [--check]         check for a release, or download and install it
pinfold config [ROOT] [--json]   selected project settings and image
pinfold config edit              create this project's configuration file if missing, and open it in $VISUAL or $EDITOR
git fetch box / git push box     a disk project's branches and tags, through git-remote-pinfold (Git)
pinfold box …                    boxes from a box spec (Process interface)
pinfold image build|rm NAME … [--json]   a caller's image, built from its own context, or retired (Process interface)
pinfold init                     PID 1 in the box (Linux builds)
```

`pinfold box …` and `pinfold image …` are the developer interface, the
Process interface, for integrators and the test suite.

`doctor --artifacts --json` prints one JSON array: the host `codex` helper, then on Linux
entries named `vmm` and `builder`: `name`, `version`, `path` (the helper
executable or infrastructure install directory), `cached`, and
`assets`, a list of `{url, sha256}` for this host's `os-arch`. Nothing is
downloaded. The builder asset includes `url`, `sha256` and `bytes`; its
entry also reports the recipe identity. Without `--json`, `doctor --artifacts`
prints a readable inventory. Plain `doctor` leads with findings and remedies;
it names an outdated chain once, as its lowest outdated image, the count of
images built on it and the build, where `--json` lists each image.
In a configured project its `Logins` line names each effective login route
with its agent, and Pi's provider, or says there is none. It reads
configuration only: no value, and not whether a variable is set.
`doctor --json` prints the diagnostic report as structured data. Its
`config` has the `config --json` report's shape, or is null when doctor
has no project report. Its `egress.routes` holds each route as
configured, a login route with its `login`.

`pinfold --version` (`-V`) and `pinfold --help` (`-h`) answer without
touching the runtime or the state directory. Bare `pinfold` reports the
project root, box status, profile, project mount and settings path, and
the image only when it is missing. With no configuration it previews the
profile and the default mount the first run asks to confirm. It ends with
next steps and succeeds. It reads project and owner information and image
availability without creating state or starting a box.
Explicit help groups the command inventory
by task, one line per top-level command, with advanced commands, the
developer interface among them, last. Report commands use human output unless `--json` is requested,
including when stdout is redirected. When the reader of stdout closes early,
`box`, `disk`, `image`, `clean`, `build` and `update` finish their work
quietly with their own exit status; other commands, and every command's
help, die of SIGPIPE, or exit 141 where it is blocked, without a
diagnostic. The box protocol and guest stdio retain
their own output contracts. Unknown top-level commands name the
command and point to help. Editor failure reports the status and how to open
the configuration file. `--help` or `-h` after a subcommand, before any
`--`, prints that subcommand's syntax lines and exits 0, touching neither.
The first top-level `--` introduces literal command argv; subsequent
arguments are never interpreted as pinfold help. Bare `pinfold --` or an
empty command name exits 125 before project setup. Arguments must be UTF-8.
Host paths in human diagnostics are control-character escaped; JSON uses
normal JSON serialization.

`start` and `stop` take no arguments and print nothing to stdout on success.
They return 0 on success or 125 for a project infrastructure failure; host
signals retain `128+n`. `stop` and the private owner entrypoint run no automatic
maintenance or update notices.

Project discovery (Terms) reads metadata only. No configuration is parsed before a live
owner compares host roots; changed roots refuse as `config-changed` before
configuration or worktree errors. For a share, topology is established by the
Git-metadata control above. Inside a Git directory, the command refuses as
`project-in-git`.

## Guarantees

Each row is one test in the end-to-end suite.
"Shown by" names its hardest scenario and the positive control in the same
run. Testing policy is in `AGENTS.md`. Row numbers are stable; gaps are
cut rows.

| # | Guarantee | Shown by |
|---|---|---|
| 1 | **No network but loopback.** The box has no interface but loopback. Its only way out is its own proxy; without `egress` it has none. | In a box with a route, only `lo` exists and the documentation address `192.0.2.1` is unreachable. A box without `egress` connects to the proxy's port and is refused. On Linux, vsock connections to CID 2 on every port from 1 to 4096 are reset at once, not timed out: all of them in the box without `egress`, all but 3128 in the box with a route. Control: the same image with a route reaches the host fixture through the proxy; on Linux, in that box, CID 2 port 3128 reaches the proxy. |
| 2 | **Only allowlisted hosts get through.** The proxy lets out only allowlisted names, never an IP literal or a name resolving to a refused address (Egress proxy). A route reaches exactly its one host service, and its log line names the method, the path without its query and the upstream status. The proxy answers `Expect: 100-continue` itself and refuses any other expectation. A refusal's response names the reason its log line records. | `localhost` and the host's own address are put on the allowlist and requested; each gets a 403, logged `loopback` and `ip literal`. A CONNECT to an allowlisted name whose ClientHello SNI names another host is accepted, then refused and logged `sni mismatch`. `example.com` gets a 403 logged `not allowlisted`, over CONNECT and plain HTTP. For each of these 403s, the `Proxy-Status` `details` curl shows in the box equals the logged reason. Control: in the same box, `api.github.com` answers, and a route reaches the host fixture with the Host header rewritten; its log line names the method, the path without its query and the upstream status. A POST to the route with `Expect: 100-continue`, from a curl that waits longer for `100 Continue` than its own bound, reaches a second host fixture that answers 200 only to the body the box sent, and that fixture receives no `Expect`; one with another expectation gets 417 logged `expectation unsupported`, whose `Proxy-Status` `details` equals that reason, and reaches no fixture. |
| 5 | **Nothing can gain privileges.** Box work runs as the host uid with no capabilities and cannot regain privileges, in a VM with its own kernel. | In a box on the built-in `default` image, a grandchild of an exec'd command (fork then exec) has the host uid, `CapBnd` 0 and `NoNewPrivs` 1, and its `unshare -U` fails with `ENOSPC`. Box work can reach no image-provided setuid or setgid file: there is none in any directory it can enter, and each directory it cannot enter refuses it. The box's root is not writable. On Linux, PID 1 keeps only `CAP_SYS_ADMIN`, and the exec service is the host uid with every capability set empty. Control: a plain child runs in the same box, and the grandchild's `unshare(0)` succeeds. Two boxes report different `/proc/sys/kernel/random/boot_id` values, and on Linux each differs from the host's, so each box boots its own kernel. |
| 8 | **Losing the owner fails closed.** If the owner dies, the box loses its way out, and the leftover can be removed. | SIGKILL of `box up`. On Apple the guest survives: `box exec` runs a script in it that prints a marker and then requests an allowlisted host; the marker appears and the request gets no upstream response. On Linux the VM ends with its owner (43). Another state root sees the owner dead, and `box prune` in a state root whose daily pass is due removes the leftover; the name is reusable. SIGKILL of a detached project owner while a session runs makes the client exit 125. Control: before the kill, another state root lists the owner alive, and on Apple the same in-box request succeeds. |
| 10 | **Disk-project branches and tags cross only through the Git remote.** A fetch writes box branches only under `refs/remotes/box/*` and adds only new tags; it never moves or deletes the host's own branches or tags, except as the threat model lists. A push lands only in the box's `refs/remotes/host/*` and its tags. Crossing carries no hook or configuration. | (a) A box that writes its own listing names a branch at a host-only commit. `git fetch box` fails naming that branch, no ref under `refs/remotes/box/` changes, and `.git/objects` is unchanged. So does a bundle whose pack also carries an unreachable tag of a tag of a missing commit, failing naming that commit; and, with the host a partial clone of a local promisor, a listing that names the branch at a commit only the promisor holds and a bundle naming it as a prerequisite. (b) Separately, a valid listing carries a tag named like a host tag at another object. After `git fetch --force --tags --prune --prune-tags box`, every host tag is where it was. The box then commits hooks under every client-side name, `core.hooksPath`, `.husky/` and `.githooks/`; the host fetches, merges and checks out twice, and nothing planted runs. With `push.followTags`, `git push box main` lands main only in the box's `refs/remotes/host/main` and its annotated tags as box tags, and changes no box working-tree file; after it and after a repeat, the host's `refs/remotes/box/main` is the box's own main. A fresh start leaves the project disk empty and the host's `.git/config` byte-identical; the first push creates the box repository. Control: a host hook runs, and a legitimate branch and new tag land under `refs/remotes/box/*` and `refs/tags/*`. |
| 11 | **Disks persist and attach exclusively.** A disk outlives its box. One box at a time writes it, and a disk left unclean is not read until a writable run recovers it. | Two writable `up`s race for one new disk: exactly one runs, and the other is refused `disk-in-use`. After the writer's owner is SIGKILLed, a read-only attachment is refused `disk-in-use` while the dead box's claim stands; once `box prune` retires it, the same attachment is refused `disk-unclean` until a writable run ends. `disk rm` of an attached disk is refused `disk-in-use`. Control: a file one box writes is read by the next box that attaches the disk. |
| 15 | **Cleanup removes only pinfold's garbage.** `clean` never removes a live project's state or disks, or host files. | A kept project with a live box and no clients, beside an idle project. `clean --idle 0s` removes the idle project's state and disk and keeps the live one's: a write still reads back through its box. The idle project's checkout tree and configuration file are unchanged. Control: the idle project's disk is gone. |
| 26 | **Credentials stay on the host.** pinfold adds a route's credential at the proxy and never puts it in the box; an upstream that returns it can still hand it back (threat model). A `from`, `op` or `PINFOLD_ENV_` value reaches the box's environment, never host argv, the runtime's records or a file pinfold writes. | One box with an injecting route, a Claude login route and a Pi `openrouter` login route, each pointed at a host fixture that echoes no credential, plus a `from` variable, an `op` env entry and a `PINFOLD_ENV_` value. A stand-in `op` first on the client's PATH gives the Claude route's token, which a second injected header names too, and the env entry's value; it is asked once for each reference. Before the session, a launch whose configuration differs from the session's only in the Claude route's reference, one the stand-in refuses (printing a decoy on stdout and stderr), exits 125 with empty stdout, its stderr naming the reference and not holding the decoy, the stand-in logged that reference, and `box list` shows no box for the project. Through the injecting route the guest sends its own value for the injected header, in another letter case; the fixture receives only the host's, and the second header the `op` value. Through the Pi login route the guest sends mixed-case duplicate `Authorization` and `X-Api-Key` fields; the fixture receives only the host credential, and an off-list field no `Connection` field names arrives unchanged. A later request whose `Connection` field nominates that field and `Authorization` delivers the host credential without the nominated field. A `?api_key=` query on that login route gets 403 `login credential query` and no upstream request. No route credential is in the box's environment, any file it can read, its home disk or the egress log. On the host no secret, plain or hex, is in any file under the state, config or cache roots or in any process argv; on Apple, neither in `container inspect`, the container's directory nor the box's new volumes. A configuration literal is in its own file and no other file under those roots. Control: the same login request without the query succeeds in the same box; the box search finds a canary on the home disk, and `ps` finds the session's argv canary; the `from`, `op` and `PINFOLD_ENV_` values arrive in the box's environment; the session that differs only in the refused reference starts; Claude and Pi each complete one fixture exchange through their generated configuration. |
| 38 | **Every egress decision is recorded, or the box stops.** Each run has its own egress log and launch record. The launch record is the policy applied at start. | The live egress log is padded to one byte below its 128 MiB cap. The next route request stops the owner with `audit-log` and exit 1, and the log's length is unchanged. Control: in a second run of the same name under another allowlist and memory, an allowed and a refused request each appear once in that run's log, and the first run's log still holds only its own; `inspect` answers with no runtime on PATH. Each run's launch record names the allowlist, routes, cpus and memory it applied, and env names without values. |
| 43 | **The VMM reaches only its own box.** Linux only. Each VMM runs as the user in its own jail and sees only its own box's files. | Two boxes up, one with a detached owner. Each VMM process is the user's, with `CapBnd` 0 and `NoNewPrivs` 1, in user, mount, PID, network, IPC and UTS namespaces of its own. Through `/proc/PID/root` it sees its own disk and not the other box's; no directory from the home is mounted in it; its jail holds no socket but those in its own vsock socket directory, so no project session or shutdown control socket; every descriptor it holds is in its jail, `/dev/null` or an anonymous kernel object. Control: its own disk is visible, and SIGKILL of the owner ends the VM. |
| 44 | **Linux image work stays in its own VM.** Linux only. A build's RUN steps have no network but the production proxy, which lets out only allowed hosts and audits each decision. | In one build, a RUN step's direct connection to a public address, bypassing the proxy, fails; through the proxy `api.github.com` answers while `example.com` gets a 403, with the denial in the build's egress log. Control: the same build with `--allow example.com` reaches it. |
| 45 | **Disk export writes only a new destination.** Export copies a stopped disk's files into a new directory, never over an existing one, and follows no box-planted link. | A stopped disk holding a symlink to a host path and an executable file is exported. The link is copied as a link and its target is untouched; the file stays executable. Export to an existing destination, a dangling symlink, is refused `destination-exists` and leaves the link unchanged. Control: export into a new destination succeeds. |
| 47 | **A project's sessions share one box.** All of a project's sessions use one box under its startup configuration. One session ending does not end another, and a client with different configuration is refused. Only the launch that starts the box runs `op`. | Two simultaneous first launches, each with a stand-in `op` first on PATH that gives the reference's value, share one owner and generation, and that stand-in is asked once; the box has its value. Killing one client ends its command group while the other keeps working, and SIGKILL of a fetch's Git alone ends the box command it hung, even one that ignores TERM. A client whose `from` variable holds another value is refused `config-changed` naming `env.values`, and one whose `op` reference differs naming `env.names`. Every later client of the project (the same-value join, the two refused ones, and the exit-code and executable-lookup runs) has instead a stand-in `op` that refuses every read first on PATH, and none asks it. After the configuration file's allowlist changes mid-run, the started allowlist still decides. Commands get literal argv, empty arguments and `--help` included; exits 0, 23, 126 and 127 pass through, and a setup failure exits 125. With no configuration and no terminal, `-- CMD` is refused `no-config` and creates no project state, and `git fetch box` from an unused state root is refused `no-config` and leaves that root empty. Control: the same `from` value joins, after `config edit` the refused invocation runs, and a configured disk project's `git fetch box` from that root passes admission and runs the daily pass. |
| 48 | **A share reaches only its folder by path, a read-only folder is read-only, and a project holding a protected path is refused.** Through a share the box reaches no host path outside its declared directory; a hard link inside the share to an outside file is not covered (threat model). A project's read-only folder is readable in the box and not writable through it. A project holding a protected path, or with a read-only folder overlapping one, is refused. On Linux, which has no shares, the refusal runs for a disk project. | With the project's configuration directory symlinked into the checkout, `-- CMD` and `config edit` are refused `host-path-in-project` and create no state. Separately, with only the selected profile's directory symlinked in, and on Apple with only the supplied init's own directory under `artifacts/init` symlinked into a project with no configuration, the same two commands are refused the same way. A project whose read-only folder is a real directory inside the state root is refused `host-path-in-project` by `-- CMD` at the box start, with the state tree unchanged; with that folder replaced by a host fixture folder at a fresh guest path, the box reads the fixture's unique file there, a write of bytes to it fails `EROFS` and the host file is unchanged. In a running share, the box follows an outward symlink and cannot read an unmounted host fixture. Control: with each symlink removed the same project runs, and on Apple shared edits and a box `git commit` land on the host's branch. |
| 50 | **A profile with a base applies the base beneath it and is outdated when the base changes.** | A three-link user chain of tiny recipes, none with `base = "default"`: the root is on a public base on Linux and on the already-built default image on macOS. After a change to the root's recipe, a launch names `image-outdated` for the root and every image above it; after rebuilding only the root, for the two above it; after the chain's rebuild, none, and a second unchanged rebuild keeps every image identity in the chain. Control: a box from the top link has every link's file and seed and a top-level replacement seed. With the chain built, a cycle is refused as `profile`, and correcting it runs. |

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

Dependencies: `tokio`, `httparse`, `serde`, `serde_json`, `toml`, `nix`
(with `socket` for vsock, `net` for the proxy's upstream addresses,
`sched` and `mount` for the VMM jail; Landlock
through nix's `libc` syscalls),
`ring` for host SHA-256, `sha2` only in the guest builder init, and the TLS client for
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
