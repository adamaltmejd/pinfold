# pinfold architecture

pinfold runs a coding agent in a disposable box: a micro-VM, Apple
`container` on macOS and Firecracker on Linux. The box sees its
disks and read-only mounts and nothing else of the host. Its only way out is
its own allowlisting proxy; its work reaches the host only as git
branches, through `fetch`.

## Terms

**Box**: one disposable micro-VM, described by a box spec and
owned by one `box up` process.
_Avoid_: sandbox, container, VM, run

**Box spec**: the JSON a caller gives `box up`. It is the whole box: nothing
is added that the spec does not name.

**Project**: a checkout, identified by the path of its root. A moved
checkout is a new project.
_Avoid_: repo, workspace

**Profile**: data that shapes a box: an image, seeds, read-only shared
files, and config defaults.

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
| pi | shim, per-project state and disks, config, trust, herdr | the `pinfold` CLI |
| profiles | box defaults, an image, pi's user-level config | applied as data |

Core knows nothing of pi; its only git is the branch crossing (Disks). To
core, a profile's pi config is files.
Core never reads a profile's `pinfold.toml`; the pi layer does, when it
writes a box spec. Profiles hold no code except pi packages.

## Threat model

A prompt injection turns the agent into an exfiltration path or into code
running on the host.

| Control | Effect |
|---|---|
| No network | No interface but loopback. One socket to the box's own proxy (vsock on Linux). |
| Egress proxy | Default deny, allowlist by name, every decision logged. |
| Kernel boundary | A VM per box, with its own guest kernel. |
| No privilege | Box work runs as the host uid with an empty capability bounding set; read-only root, no setuid files. On Linux the guest's PID 1 alone stays root, holding only `CAP_SYS_ADMIN` after boot, to unmount disks at `down`. |
| VMM jail | On Linux the VMM runs as the user in a jail `box up` builds: it reaches only its own VM's files. |
| Host files | A box writes no host file. Its work leaves only by `fetch`: git refs under one remote namespace, no guest hook or config. |
| Profiles | Nothing a box can write maps to a profile. Only host-side commands change one. |

Not protected:
- Project contents: an agent with a model API can put them in a prompt.
- What the host does with fetched refs: merging, checking out, building
  or opening them runs the box's content as the user. A merged
  `.husky/pre-commit`, editor config or planted `sub/.git`
  is the user's from then on.
- CDN fronting beyond the SNI check (Host-header fronting needs TLS
  interception).
- Allowlisted services that accept writes (GitHub with a token, registries).
- Image builds: user- or caller-run, trusted, unrestricted egress.

## Runtimes

One runtime per OS, not configurable:

| Host | Runtime | Kernel |
|---|---|---|
| macOS 26+ | Apple `container` 1.6.0+ (stable) | one VM per box |
| Linux | Firecracker, pinned (Pinned artifacts), run as the user in pinfold's VMM jail | one VM per box, pinned guest kernel |

Anything else is refused: podman or docker as a box runtime, Firecracker
under its own jailer or outside pinfold's VMM jail. Nothing warns at run
time; `doctor` reports the runtime. On Linux, rootless podman 6.1.4+ builds images.

Preflight refuses box startup on older, prerelease or unrecognized runtime
versions. Inventory and cleanup remain available on older versions.

Linux is x64 only. Preflight requires a user other than root, `/dev/kvm`
readable and writable by the user (the `kvm` group), unprivileged user
namespaces, Landlock ABI 5 or later (Linux 6.10+), `mkfs.ext4`
(e2fsprogs), and podman with `rootless=true`. Without user
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
   Record a fresh generation, owner pid and state directory; label the box
   with that generation. Claim acquisition waits up to ten seconds through
   contention, including maintenance probes, and is signal-cancellable.
   A name still held at the deadline is `name-in-use`; abandoned partial
   claims are reclaimable. Nothing (seeds, disks, proxy, box) is created
   before the claim.
2. Claim the box's disks (Disks), creating any that are new; a new
   disk's `from` is size-checked here. On Apple a transient container of
   the box's image, root with only `CAP_CHOWN` and no network, gives each
   new disk's root to the box user. It carries the box's name and its
   owner and generation labels, so `clean` removes a leftover.
3. Start the box's proxy on a new unix socket in the state dir. Keep the path
   under macOS's 104-byte limit.
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
   ready, the box copies each new disk's `from` in as the box user, and on
   Linux the profile's missing home seeds.
6. Apple only: wait until the runtime lists the box running, including
   without egress. One transient root exec makes the socket connectable,
   when there is one.
7. `exec` work runs with `HTTPS_PROXY` and `http_proxy` set to
   `http://127.0.0.1:3128`.
8. Sync and unmount the disks (Linux: init, after ending box work with
   the work cgroup's `cgroup.kill`; Apple: the runtime's stop),
   stop and remove the box, record the disks clean and release them, and
   delete the socket.

One `pinfold box up` process owns one box. It does steps 1–6, holds the
proxy, and does step 8 on `down`, stdin EOF or SIGTERM. If it dies, the
socket dies and the box has no way out (fails closed); `box prune` removes
leftovers host-wide. The CLI runs the same code in-process.

### Process interface

```
pinfold box up < spec.json        # prints ready, holds the box, ends with down
pinfold box exec BOX [--tty] [--workdir D] -- argv…   # stdio through, exit code back
pinfold box stat BOX              # one JSON object of the box's memory, pids and OOM kills
pinfold box down BOX              # asks its owner to tear down; an absent box exits 0
pinfold box list --label KEY[=VALUE]…      # JSON lines; repeat --label to AND filters
pinfold box prune                 # remove boxes whose `up` is gone, print each removed
pinfold box inspect RUN           # one JSON object: a run's retained records
pinfold box events RUN [--limit N] [--after CURSOR]   # one JSON object: a page of a run's egress log
pinfold box push BOX --repo REPO --into GUEST_REPO [--remote NAME]   # host branches into the box; one JSON line
pinfold box fetch BOX --repo GUEST_REPO --into REPO [--remote NAME]   # box branches into REPO; one JSON line
pinfold disk list                 # JSON lines
pinfold disk rm NAME              # one JSON line
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   # one JSON line
pinfold image rm NAME             # one JSON line
```

`image build` builds a caller image (see Images) and prints one line on
stdout; the build's progress never reaches stdout. On success, exit 0:

```json
{"event":"built","image":NAME,"ref":"localhost/pinfold/image-<NAME>:<build>","latest":"localhost/pinfold/image-<NAME>:latest","id":ID,"labels":{…},"base":DIGEST|null}
```

`id` is the image's runtime id, the same value a box's `ready` and `list`
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

On a box that is absent, `exec` exits 3 with `pinfold box exec: no box
named ...`; any other exit code is the command's. `push` and `fetch`
(see Disks) exit 3 the same way.

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
that label itself (a pi box's `dev.pinfold.project`). `egress_log` is the
absolute path of the box's egress log (see Egress proxy), null without
`egress`.

The caller keeps `up`'s stdin open for the life of the box; closing it is
`down`. The stream ends with one `down` line after teardown:

```json
{"event":"down","box":NAME,"run_id":ID|null,"reason":REASON}
```

`REASON` is one of `stdin-closed`, `signal` (SIGTERM, SIGINT or an accepted
`box down` request), `audit-log` (an egress log append failed), or `exited` (the box's init ended on its own;
`detail` carries its exit status). `up` exits 0 for `stdin-closed` and
`signal`, and 1 for `exited` or `audit-log`; `box` is null when a signal ended `up` before
its spec parsed, and `run_id` is null when it ended `up` before `ready`.

When `up` refuses, it prints one JSON line instead of `ready` and exits 1:

```json
{"event":"refused","box":NAME,"reason":REASON,"detail":TEXT}
```

`REASON` is `spec`, `profile`, `runtime`, `image-missing`, `name-in-use`,
`login`, `disk-in-use`, `disk-unclean` or `limit` (see Disks);
`box` is the `name` of stdin's first JSON value when that is an object with
a string `name`, else null.
Early refusals create no box, box state, disk, seeds or extracted profile
resources. `disk-in-use`, `disk-unclean` and `limit` are decided after
the claim; they release it and leave no state. Failed-start cleanup removes claimed
state and any disk this `up` created. Fetched
artifacts and ordinary home seeds may already exist and are not rolled back.
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

`down` skips daily maintenance. It uses a host-only control socket and addresses
the observed generation. The owner acknowledges after teardown. A timed-out request
fails without releasing ownership or removing a live owner's state.
Orphan removal and `prune` acquire the lifetime lock and recheck the
generation before removing anything; an old observation cannot remove a
replacement. Control sockets never enter boxes. The checked per-user lock
directory is `/private/tmp/pinfold-<uid>` on macOS and
`/tmp/pinfold-<uid>` on Linux. Persistent lock files are never unlinked.

`list` prints one JSON line per pinfold box whose labels match every `--label`.
It skips daily maintenance. On Linux a box is a claim whose VMM has started; `list`
reads pinfold's own claims and whether each VMM still runs, and asks no
runtime. On Apple it reads `container list`.
`--label KEY=VALUE` matches that value; `--label KEY` matches any value of
`KEY`; every box carries `dev.pinfold.owner`, so `--label dev.pinfold.owner`
lists them all. A container without it is no box: `exec` and `stat` exit 3
on it and `down` leaves it. Each line carries the box's labels and its
image, then its `dev.pinfold.owner` pid (or null), whether its
generation owns the lifetime lock, and the runtime's RFC
3339 `created` time and `state` (`running` or `stopped`):

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
  "harness": "pi",
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
- `env` is exact; nothing is inherited. An env name must match
  `[A-Za-z_][A-Za-z0-9_]*`; any other name is refused as `spec`, naming it.
  `{ "from": "NAME" }` reads the caller's value. Guest values travel under
  private transport names and are restored by `init exec`, so they cannot
  configure the host runtime client. Values never reach argv or files;
  multiline values remain intact. The runtime executable is found on the
  host's `PATH`, never the spec's.
- A label key starting with `dev.pinfold.` is refused as `spec`, naming
  it: the namespace is pinfold's.
- `profile` applies the profile's `home/` and `share/` (see Profiles), and
  its image if `image` is absent; egress, env and resources come only
  from the spec.
- `harness` is `pi`, `claude` or `codex` (see Pinned artifacts); any other
  value is refused as `spec`, naming it and the accepted names. It mounts
  that harness's directory read-only at `/opt/pinfold/<name>`, its
  executable `/opt/pinfold/<name>/<name>`, and sets its pinned env
  defaults and `PINFOLD_ALLOW` (the spec's `egress.allow`, empty without egress). The
  spec's own `env` wins; a spec mount at `/opt/pinfold/<name>` is refused.
  claude is the glibc build, so it needs a glibc image (the default profile
  is Debian). codex's `codex-code-mode-host` sits beside it. codex's own sandbox does
  not start in a box; a caller runs codex with
  `sandbox_mode = "danger-full-access"`.
- Every mount is read-only and shows its host directory as of `up`; a
  later host change may or may not appear. A box writes only to its disks
  and its tmpfs `/tmp`.
- Mount hosts must resolve to existing directories (symlinks to directories
  are accepted); a host path that cannot be read or is not a directory is
  refused as `spec`, naming it. A mount or disk nested in another applies
  inside it whatever the spec's order; two at one lexical guest path, with
  `.` and `..` resolved, are refused as `spec` before preparation.
- On Linux a box attaches at most 24 disks and mounts, pinfold's own
  (the harness, `share/`, codex's config) included; more are refused as
  `spec`.
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

The launch record is written once, atomically, after the box accepts
`exec` (Lifecycle step 6) and immediately before `ready`. Failed-start
cleanup removes the run's directory:

```json
{"run_id":ID,"box":NAME,"time":RFC3339,"pinfold":VERSION,"runtime":{"name":"apple"|"firecracker","version":V},"image":{"id":ID,"ref":REF},"profile":NAME|null,"harness":{"name":NAME,"version":V}|null,"labels":{…},"mounts":[{"host":PATH,"guest":PATH}],"disks":[{"name":NAME,"guest":PATH,"readonly":BOOL}],"env":[NAME,…],"egress":{"allow":[…],"routes":{NAME:{"kind":"host-service"|"injecting"|"login","to":TARGET}}}|null,"cpus":N,"memory":M}
```

Every field is a value the box spec carries or that `ready`, `list`,
`artifacts` or `doctor` already prints. It never holds an env value, a header name,
value or prefix, a login token, argv or command output. `env` lists names
only. A route's `to` is its `host:port` or origin. The record is what this
run applied at start; a later change to config or trust leaves it as it is.

The terminal record is written once, after teardown, before the owner
acknowledges `box down` and before `up` prints `down`:

```json
{"run_id":ID,"time":RFC3339,"reason":REASON,"detail":TEXT|null}
```

`reason` and `detail` are the `down` line's. A pi box, which prints no
`down`, adds `harness-exited` for its harness ending, with its exit status
in `detail`. An owner that dies without
teardown writes none, and nothing infers one later: `prune` writes no
terminal record.

The egress log append rule (Egress proxy) is unchanged; it is the only
write failure that stops a running box. A launch record that cannot be
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

Both queries skip daily maintenance, take no lifetime lock, start no runtime client
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
- Apple adds: `--network none`, `--cap-drop ALL`, `--read-only`. PID 1 runs
  as the host uid.
- Firecracker adds: no network device; the root disk and init disk
  read-only; `vcpu_count` and `mem_size_mib` from `cpus` and `memory`, 4 and 1G
  when the spec names none, as Apple's defaults; no swap; the image's
  `Env` as defaults under the spec's `env`; an empty `/etc/resolv.conf`;
  Firecracker's own seccomp filters. `box up` runs the VMM as the user in
  the VMM jail with a parent-death signal, so the VM ends with its owner.
- In the Firecracker guest, PID 1 sets `user.max_user_namespaces` to 0
  and forks the exec service as the host uid with every capability set
  empty, in a root-owned work cgroup that holds the pids cap and that box
  work cannot leave, and keeps only `CAP_SYS_ADMIN` itself. PID 1 then
  only reaps and answers Lifecycle step 8's unmount; it handles no
  workload bytes.
- pinfold puts no secret value in a kernel command line, a VM
  configuration, an image layer, a disk or a persisted request; what a
  guest process writes is its own. On Linux the spec's env reaches
  the guest only over the exec connection, after boot, and the exec
  service holds it in memory.

Runtime command lines and VM configurations are built as data.

### VMM jail

On Linux `box up` builds each VMM's jail itself, with no privileged helper,
before it execs Firecracker:
- new user, mount, PID, network, IPC and UTS namespaces; only the host uid
  and gid map, each to itself;
- `pivot_root` into an empty tmpfs holding only the Firecracker binary,
  the guest kernel, the root and init disks, the box's disks and the
  mounts' images (read-only binds, but for the box's writable disks),
  `/dev/kvm`, the vsock socket directory and the VM configuration file
  (Firecracker runs with `--no-api`); the old root is detached;
- a capability bounding set of zero, `no_new_privs`, and the user's
  supplementary groups, which reach nothing in the jail (dropping them
  needs privilege);
- a Landlock ruleset on ABI 5 or later (so `/dev/kvm`'s ioctls are
  explicit) that handles every access right pinfold knows up to the
  running ABI, TCP included; a newer ABI runs normally, and its new rights
  stay unhandled. It allows only: read and execute on the binary; read on
  the kernel, the root and init disks, the mounts' images, the box's
  read-only disks and the configuration; read and write on the box's
  writable disks; read, write and ioctl on `/dev/kvm`; making
  and connecting to sockets (ABI 9's `RESOLVE_UNIX`), removing files and
  reading the directory in the vsock socket directory.

Firecracker's own seccomp filters then apply.

A compromised VMM reaches only that tree, as the user: no home, no other
box's disks or socket, no host network or abstract socket, no host
process. Getting further needs a host kernel bug.

### Process supervision

- Always `-i`. `-t` only when stdin and stdout are both terminals, with the
  host `TERM` and `COLORTERM`.
- TTY: the exec client (Apple's runtime exec; on Linux, pinfold's own over
  vsock) stays in pinfold's foreground process group, handles raw mode and
  forwards window size. No TTY: its own process group.
- The box is the unit of lifetime; pinfold never signals a process in it
  from the host or through `exec`. At `down` on Linux, init kills the
  work cgroup so the disks can unmount; the box is ending anyway.
  Once pi runs on a TTY, Ctrl-C and Ctrl-\ reach it as bytes. A closed
  terminal, SIGTERM, SIGHUP or SIGINT removes the box.
  Revisit when apple/container#1941 lands: add a graceful SIGTERM before
  removal.
- Exit codes pass through; `128+n` for a signal death.
- Every process started in a box through `exec` runs with `oom_score_adj`
  1000.
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
  box user; the root exec in step 6 chmods it.

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
- **Routes:** a name maps to one host service, e.g.
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
- **Login routes:** a route whose value is `{ "login": HARNESS }` carries
  a subscription login for the box's harness; pinfold owns the origin, the
  headers and the harness's config for the pinned version. `login` must
  name the spec's `harness`, and a spec has at most one login route; else
  `spec`. An optional `to` replaces the harness's origin, checked like an
  injecting route's. The box gets only placeholders; the spec's own `env`
  wins.
  - `claude`: `{ "login": "claude", "from": VAR }`, where `$VAR` is a
    `claude setup-token` token read once at start like an injecting route's.
    The route goes to `https://api.anthropic.com` with `Authorization:
    Bearer`. The box gets `ANTHROPIC_BASE_URL=http://<route>` and a
    placeholder `CLAUDE_CODE_OAUTH_TOKEN`.
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
  - `up` refuses a login it cannot use (no token, or not a usable JWT) as
    `login`, naming the harness.
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
  An append failure stops the box with `audit-log`. Storage failure can
  prevent recording a request already forwarded to its upstream.

## Images

An image is a toolchain; the harness is not baked in. The default comes from
the profile's Containerfile. A project can name its own, typically `FROM` the
profile image.

- Images are built only by `pinfold build` and `pinfold image build`.
  `pinfold pi` refuses when the image is missing and names the command.
- A profile or project build gets an empty context: the Containerfile
  alone. Files come in
  by `ADD --checksum` or from the profile image. Every step reruns at every
  build (no layer cache).
- A build's identity is its unique tag, never an image label. A caller
  build whose layers and labels are unchanged returns the existing image.
  Every build sets `dev.pinfold.profile`, `dev.pinfold.project` and
  `dev.pinfold.base`, the family labels not its own empty. Every build is
  tagged uniquely `<repo>:<build>` and moves the stable `<repo>:latest`
  to it.
- A profile build uses `localhost/pinfold/profile-<name>`. The stable ref is
  what a project Containerfile `FROM`s and what `doctor` compares against.
  Its `dev.pinfold.profile` is `<name>` and its `dev.pinfold.base` the
  resolved base's digest.
- A caller image is built by `pinfold image build NAME` from a context
  directory the caller vouches for, as it vouches for its mounts: no trust
  record, no project, no `.pinfold.toml`. The Containerfile may lie outside
  the context. NAME is validated like a profile name. The repo is
  `localhost/pinfold/image-<NAME>`; a box spec's `image` names either tag.
  Identical inputs under two names build one image. It carries
  `dev.pinfold.base` (the first `FROM`'s digest when it resolves to one,
  else empty; a base pinfold built is read from local storage, never
  pulled), both family labels empty, and the caller's `--label`s, none of
  which may start with `dev.pinfold.`. It uses
  the runtime's layer cache unless the caller passes `--no-cache`; podman
  labels the cache's intermediate images `dev.pinfold.layer`.
- A project build uses `localhost/pinfold/project-<id>`, where `<id>` is the
  project's state id. A project image carries `dev.pinfold.project=<id>`,
  and records the digest of the profile image it was built from as
  `dev.pinfold.base`.
- A build never touches project homes.
- Profile and project images record their Containerfile's SHA-256 as
  `dev.pinfold.containerfile-sha256`. Before a pi launch, and in doctor,
  changed or unrecorded Containerfile inputs produce an `image-outdated`
  hint with the rebuild command. A project checks its selected profile's
  inputs too, even before that profile is rebuilt. A changed profile image
  digest also names `image-outdated`. Hints never refuse the launch.
  Harness pins and home settings are not image inputs.
- On Apple, concurrent pinfold builds do not interrupt each other when
  callers set different `NO_COLOR` or `BUILDKIT_COLORS`.
- Apple base refresh pulls only the host platform unless the first `FROM`
  names an explicit platform. An unresolved platform is not pulled.
  The empty `scratch` base has no digest and is not pulled.

On Linux a box boots its image as a read-only ext4 root disk: podman
exports the image and `mkfs.ext4 -d` writes it, inside podman's user
namespace so files keep their image owners. The disk is made at the first
`up` of an image id, at `~/.cache/pinfold/root/<id>.ext4`, and goes when
the image goes.

Built-in images use `debian:trixie-slim`, upgrade Debian packages at each
build, record the resolved base digest and strip setuid/setgid bits.

| Profile | Tools |
|---|---|
| `default` | shell, ca-certificates, git, curl, ripgrep, fd-find as `fd`, jq, less |
| `documents` | default tools plus Bun, AnyDoc and Poppler |
| `full` | documents tools plus gh, rtk and ponytail |

Fixed recipe fragments under `profile/image/` form complete independent
Containerfiles. A built-in never builds from another profile's image.
Release assets use pinned `ADD --checksum` inputs for the selected
architecture. Final images contain installed tools, without their archives.
gh comes from GitHub's signed apt repository.

## Pinned artifacts

Artifact and update downloads share a bounded HTTPS-only downloader.
It ignores curl configuration, rejects HTTP redirects, caps artifact
transfers at 512 MiB and 5 minutes, and checks SHA-256 before installation.
Cancellation kills and reaps the transfer before startup cleanup.

`crates/pinfold/harnesses.toml`, embedded at build time, pins each harness
(pi, claude, codex): a version, env defaults, and per box `os-arch`
(`linux-arm64`, `linux-x64`) its release assets (URL, sha256, install form and
path). On a box's first use of
a harness, its assets are downloaded, checked and installed into
`~/.cache/pinfold/artifacts/<name>/<version>/<os-arch>/`; its `<name>/`
directory there is mounted read-only at `/opt/pinfold/<name>`. An install
records its files; a use that finds one missing reinstalls it. A checksum
mismatch fails the start, naming the harness, version and asset. A harness
no box asks for is never downloaded.

`scripts/bump-pins.py` moves the pins: harnesses, Firecracker and the
guest kernel, image tools, Rust, Cargo dependencies, prototype packages
and the Public Suffix List. Pins move only with a pinfold release. A
version pin only moves up, to a stable version at or below upstream's
latest stable release, and is never rehashed at its version. The pi,
claude and codex harnesses take the latest release once its assets exist;
other image tools and prototype packages take a version only seven full
days after its last required asset was published, and a missing
publication time makes it ineligible. Cargo dependencies take the latest
stable, non-yanked version. Every
artifact download must match its publisher's digest; a source or
integrity failure aborts before any pin file is written.

On Linux, `crates/pinfold/vmm.toml` pins Firecracker and its guest kernel
for `linux-x64`, each checked against its pinned digest. The guest kernel
is pinfold's own build of a pinned kernel.org stable release, verified
against kernel.org's signature and published as a prerelease, never the
latest release, without
vsock loopback and with its config at `/proc/config.gz`; `guest-kernel/`
builds it. Both install like a harness, into
`~/.cache/pinfold/artifacts/vmm/<version>/<os-arch>/`, where `<version>`
names Firecracker's and the kernel's, on the first `up`.

codex also pins a host helper, `codex-app-server`, per host `os-arch`
(`darwin-arm64`, `linux-x64`), installed the same way into
`~/.cache/pinfold/artifacts/codex/<version>/host-<os-arch>/` on the first
`up` with a codex login route. It never enters a box.

The init is not an artifact: it always comes from the CLI's own build
(Code). The macOS CLI writes its embedded init to the cache once; a Linux
CLI writes its own executable into a read-only init disk.

## Profiles

A profile works the same for interactive runs and programmatic boxes,
with or without a TTY.

```
~/.config/pinfold/profiles/<name>/
  pinfold.toml    config defaults (Configuration)
  Containerfile   the profile image
  home/           seeds for $HOME
  share/          mounted read-only at /opt/pinfold/profile, as of each start
```

- `default`, `documents` and `full` are built into the binary from
  `profile/`. A user profile with the same name replaces the whole
  built-in. User profiles are the user's files and need no trust.
- `pinfold profile new` (CLI) writes a copy of `default`, or the profile
  named by `--from`. `--builtin` selects that named built-in directly,
  bypassing a user override; without `--from` it selects `default`.
- `home/` needs `$HOME` on a writable disk; seeds (Terms) are copied in at
  each start.
- pinfold does not manage pi settings: `pi install` in a box changes that
  project home only.

pi's two config levels both load in every run:
- **User level:** pi's agent dir under `$HOME`, seeded from `home/`, with
  packages from `share/`.
- **Project level:** the project's own `.pi/` and `.agents/skills`,
  pi-native.

Every built-in seeds only `/opt/pinfold/profile/pi` as its pi package and
sets `defaultProjectTrust: "always"`: the box is the boundary. It also
adds pi's built-in `codemode` tool to the default tools; pi's built-in MCP
is on by default: stdio servers run in the box, and remote ones are
reached through the proxy. The selected
profile's live package chooses its extensions and skills, so switching
profiles does not rewrite saved settings. Every built-in includes the operating-context
extension, which writes the box's facts into the system prompt (the
allowlist from `PINFOLD_ALLOW`, that a 403 is final, and that its commits
reach the host only when the user runs `pinfold fetch`). Documents and full include `read-documents`. Full also
registers rtk and ponytail. Metadata lookup and fingerprints write
nothing; embedded shared files are materialized only after a box claims
its name.

The default and documents profiles' `pinfold.toml` set no config, so
they get the built-in defaults. Full's sets `allow` to the default list
plus `github.com` and `api.github.com`. `PI_OFFLINE` is unset, so pi's
package installs go through the proxy.

## Disks

On Linux a disk is an ext4 image at
`~/.local/state/pinfold/disks/<name>.ext4`, mode 0600, created by
pinfold; on macOS it is an Apple `container` volume named
`<first 16 hex digits of the sha256 of the canonical state dir>-<name>`. The host
never mounts, reads or parses a disk's content; only boxes do. A disk's
`from` copy is written by the box that creates it, before `ready`. The
next `up` naming a disk pinfold created whose box never came ready treats
it as absent: it removes it and creates it again. Before `ready` the guest
syncs each new disk's copy. On Linux pinfold then fsyncs the disk's image,
so a disk whose box came ready holds its whole `from` after a host power
loss; on macOS what a volume keeps through one is the runtime's. In a box, a
disk's root belongs to the box user. A disk has no labels and no owner
beyond its attachments. A disk, its file or volume, its attachments and
its records belong to one state root; another root naming the same disk
gets its own.

- **Attachment:** at `up`, after the claim. One box may attach a disk writable; while it does,
  another `up` naming that disk is refused as `disk-in-use`. Boxes may
  share a disk read-only while none attaches it writable. Attachments are
  recorded under the box's claim and count while its owner lives or the
  runtime lists its box in any state but stopped.
- **Clean:** a writable attachment records the disk unclean. A stop
  pinfold requests of a running box (step 8) syncs and unmounts every
  disk, and pinfold records each clean once the runtime confirms it. A
  disk whose writable box ended on its own, or whose owner died, stays
  unclean: a read-only attachment is refused as `disk-unclean` until a
  writable one, which recovers the disk, ends with `down`.

### Crossing

Host files enter a box once, by a disk's `from`. After that, data
crosses only as git branches, through `push` and `fetch`, each against a
live box and run in it as the box user. Neither touches a working tree.
Each prints one JSON line and exits 0; a refusal prints
`{"event":"refused","box":NAME,"reason":"spec"|"limit","detail":TEXT}`
and exits 1, and a transfer that breaks off prints
`{"event":"failed","box":NAME,"detail":TEXT}` and exits 1, having changed
no ref on the receiving side.

- **`push --repo REPO --into GUEST_REPO [--remote NAME]`** bundles REPO's
  branches on the host and fetches them in the box into GUEST_REPO's
  `refs/remotes/NAME/*`, NAME `host` by default. The box merges or
  rebases them itself.
  `{"event":"pushed","box":NAME,"remote":NAME,"refs":{REF:OID,…}}`
- **`fetch --repo GUEST_REPO --into REPO [--remote NAME]`** has the box
  bundle GUEST_REPO's branches, less what REPO's `refs/remotes/NAME/*`
  already hold. On the host, pinfold runs `git bundle verify`, then
  `git index-pack --fsck-objects` of the bundle's pack, then `git fetch`
  of the bundle, both into a quarantine object directory inside REPO's.
  It force-updates `refs/remotes/NAME/*` from the box's `refs/heads/*` in
  one `update-ref` transaction, prepared before the quarantine's packs
  move into REPO and committed after, so a rejected bundle leaves REPO's
  object store as it was. pinfold removes the quarantine as the fetch
  returns; the next fetch into REPO removes one whose process is gone.
  Fetch has no deadline or memory bound: a box can stall it, or load the
  host, with a bundle slow to write or costly to index, until the caller
  kills it; a kill before the ref transaction commits changes no ref.
  A branch the bundle leaves out lands only
  at a commit `refs/remotes/NAME/*` already reaches. Every branch name,
  listed or in the bundle, must pass `git check-ref-format` under
  `refs/remotes/NAME/` before git sees it; a box with more than 4096
  branches fails. NAME is the box name by default. Tags and other refs
  stay in the box. A bundle carries no hook or config, and nothing is
  checked out; a filtered bundle fails. REPO's own hooks and config are
  the user's and apply as to any fetch. A bundle past 4 GiB is refused as
  `limit`.
  `{"event":"fetched","box":NAME,"remote":NAME,"refs":{REF:OID,…}}`

NAME is validated like a profile name. REF is the full ref set on the
receiving side.

`disk list` prints one line per disk:

```json
{"name":NAME,"size":BYTES,"allocated":BYTES,"attached":[{"box":NAME,"readonly":BOOL}],"clean":BOOL}
```

`disk rm NAME` removes a disk no box attaches, and exits 0 for an absent
one; an attached disk is refused as `disk-in-use`, exit 1:

```json
{"event":"removed","disk":NAME,"existed":BOOL}
{"event":"refused","disk":NAME,"reason":"disk-in-use"|"spec","detail":TEXT}
```

## Updates

`pinfold update --check` reports whether a newer stable GitHub release is
available. `pinfold update` downloads its host binary and `SHA256SUMS`,
verifies the binary, then atomically replaces the resolved executable.
The `pi` symlink stays intact. Failed verification or installation leaves
the installed binary intact. Releases are from `adamaltmejd/pinfold` over
HTTPS. Neither command needs a runtime or runs maintenance. No downgrade,
automatic installation or privilege escalation. An executable under
Cargo's `target/.../debug` or `target/.../release` is not replaced.

Working host commands hold a shared lock on their executable for their
lifetime. Installation requires an exclusive lock and refuses as
`update-busy` while another working command is active, including across
XDG state roots. It also refuses as `running-boxes` while the current state
root holds a live box owner from an older release. A process whose
executable was replaced before it acquired the lock must be rerun.
Checksum failures name `checksum-mismatch`; development builds name
`development-build`.

Before interactive `pi` and `attach`, with stdin, stdout and stderr all
terminals, pinfold checks at most once per 24 hours and prints an update
notice on stderr. The request has a one-second total timeout. Failed
checks are silent and count toward the interval. Help, version arguments,
development builds and noninteractive commands do not check.
Each such launch also compares all bundled profiles with their last
observed fingerprint. Changes print `bundled-profiles-changed` once,
independently of the network interval. No history means a silent baseline;
self-update records a missing baseline before replacement. Saved project
settings and user profiles are preserved. `PINFOLD_NO_UPDATE_CHECK`
(Configuration) turns off both the update check and the
`bundled-profiles-changed` notice. The timestamp and fingerprint live under
`~/.cache/pinfold/update-check`.

## Maintenance

pinfold only removes what it created: its images and boxes carry `dev.pinfold.*` labels,
and its state lives under its own dirs. Each project's state records its
checkout path and last run.

Automatic, never prompting:
- After a build: keep the newest two builds per source (a profile, a
  project or a caller image name), the second for rollback, ordered by
  their tags. Remove older builds' tags; an image goes with its last tag,
  and its dangling layers with it. An image a box still uses keeps its
  last tag, and pins only itself; which boxes use an image is read after
  dead boxes are pruned, from every box the runtime lists. A caller build
  in the last hour stays, so the ref its `built` line named still comes
  up; past the hour the newest two rule applies.
- At most once a day, at the start of any working command (not
  `--version`, `--help`, `doctor`, `config`, `artifacts`, `update`, `box list`, `box down`, `box inspect`, `box events` or `init`): prune boxes whose owner
  is gone (nothing owns its generation's lifetime lock), leftover sockets,
  artifact `<name>/<version>` directories no pin names (`init/` aside),
  builds beyond each source's newest two (the after-build rule, applied
  even when no build follows), and inactive runs' records (below).

Run records are retained by run. A run is inactive when its generation
owns no lifetime lock. An inactive run's directory goes when its newest
file is older than 14 days; then, while inactive runs together exceed
128 MiB, the oldest inactive run goes first. The quota can remove an
inactive run's egress log before 14 days. A live run's records are never
removed or truncated; its egress log can grow without bound.

`pinfold image rm NAME` retires a caller image name: it removes every tag
of NAME, whatever its age, and no other name's. The last-tag rule above
applies to an image a box uses.

`pinfold clean` lists sizes, then removes:
- everything automatic, now
- the runtime's unused build cache (Apple: prune BuildKit records, keeping
  the shared builder and active builds; podman: the
  `dev.pinfold.layer` intermediate images no image builds on)
- with `--unused AGE`, state and disks of projects not run for that long,
  except a project's with a live box. Never automatic: a project disk
  holds unfetched work, and a home disk sessions and logins.

A project whose checkout is gone or moved keeps its state and disks;
`clean` lists them, and they go only with `--unused AGE` or `disk rm`.
Other disks go only by `disk rm`. Disks are a category of their own; a
project home's caches are inside its disk and not cleaned.

`--dry-run` only lists. Run records are a category of their own. `doctor` shows disk use per category and suggests
`clean` above 20 GB. `doctor` runs no maintenance and changes no pinfold
state or artifacts. Podman's build cache is unmeasured.
When runtime checks fail, it skips runtime-dependent image and disk
probes. On Linux, failed preflight also reports missing host requirements
independently of each other: `root` names a run as uid 0, `kvm` an
inaccessible `/dev/kvm`, `mkfs` a missing `mkfs.ext4`, `userns`
unprivileged user namespaces turned off, `userns-apparmor` them restricted
by AppArmor without pinfold's profile, `landlock` no Landlock or an ABI
below 5, and `runtime-dir` the selected
`$XDG_RUNTIME_DIR` or `/run/user/<uid>` that podman builds need.

A launch holds shared project ownership before recording its run, through
box teardown. Deleting project state or caches requires exclusive project
ownership and fresh eligibility checks while holding it. A cleanup
measurement does not authorize a later deletion.

## pi layer

- **Harness:** pi, a pinned harness at `/opt/pinfold/pi`; `pinfold pi`
  asks core for it with `"harness": "pi"`.
- **State:** `~/.local/state/pinfold/projects/<name>-<hash>/` on the host,
  where the hash is of the canonical project root path, and two disks,
  `pi-<name>-<hash>-home` attached as `$HOME`, `/home/pi`, and
  `pi-<name>-<hash>-project`.
  pi's agent dir, sessions, `~/.config` and caches sit at their default
  paths in the home disk. The cosmetic name is at most 32 ASCII bytes.
  State, config and cache roots inside the project are refused as
  `host-path-in-project`, including symlinked ancestors, since the first
  start would copy them into the box.
- **Project disk:** attached writable at the project's own absolute path.
  The first start creates it with the whole checkout as its `from`:
  `.git`, untracked and ignored files included. Later starts reuse it as
  the box left it; host edits after that reach it only as branches. One pi box runs per project; a second start is refused as
  `disk-in-use`. A checkout whose `.git` is not a directory (a worktree)
  is refused as `worktree` by `pi`, `push` and `fetch`.
- **Arguments** pass through unchanged. The box starts in the invoking
  directory. Paths outside the project fail, and pi reports them. A
  non-UTF-8 path in a pi box spec or configuration report is refused as
  `path-not-utf8`, naming its field.
- **Environment:** the harness's variables (Box spec, `harness`), the
  core's proxy variables, `HERDR_AGENT` and `PINFOLD_ENV_*`.
- **TTY** whenever the host has one.
- **Auth:** OAuth `/login` once per project; API keys through the
  environment.
- **herdr:** no herdr socket. The TTY passes through, so screen detection
  works, and the shim sets `HERDR_AGENT=pi`.
- **After a run:** when the box ends for a reason other than
  `harness-exited` or `signal` (an `audit-log` stop, the box's init
  exiting), `pinfold pi` prints that reason and `pinfold box inspect
  <run_id>` on stderr after teardown.
- **Attach:** `pinfold attach [cmd…]` execs bash (or cmd) in this
  project's running pi box. It fails if none is running. Its processes end
  with the box.

### Git

The project disk holds the box's own repository. The agent commits there,
under the box's hooks and config, none of which reach the host. Built-in
images set git's system identity to `pi <pi@pinfold.invalid>`; a
repository or home identity overrides it.

`pinfold fetch` is `box fetch` into the checkout with remote `pinfold`;
the user merges; signing and host hooks are the user's. `pinfold push` is
`box push` of the checkout with remote `host`; uncommitted host edits do
not cross (Crossing).

Both check trust as `pinfold pi` does, then use the project's running pi
box, or a transient box on the project disk without egress, and print
`box push`'s and `box fetch`'s lines. With no project disk, or one whose
box never came ready (Disks), both exit nonzero and start no box.

Every `pinfold pi` start, once the box is ready and before pi runs, does
`pinfold push`'s push into it, printing nothing. It then fast-forwards
the box's checked-out branch to `host/<branch>`, by `git merge --ff-only`
in the box as the box user, only when that branch is a proper ancestor of
`host/<branch>` and the working tree has no tracked changes. Otherwise
(diverged, ahead, equal, detached, no such `host/<branch>`) the checkout
stays as the box left it. pi is not yet running; the box's own hooks
apply. The push bundles every branch's full history, so it grows with the
repository, and neither step has a deadline. A checkout with no branches
skips both silently. A failed push skips the fast-forward. A failure
prints `pinfold pi: sync-failed: DETAIL` on stderr, and pi starts.

## Experimental durable caller

`examples/pi-durable` is an opt-in Bun program. It is separate from
interactive pi and is not installed by Pinfold. Its only model tool is
sequential `boxed_bash`, dispatched through `box exec` with replay unsafe.
Reads, edits and shell commands execute in the box. The host owns model
requests, immutable job configuration and SQLite checkpoints. Recovery
holds an exclusive checkpoint-writer lock, removes the previous named box,
then creates a fresh generation before resuming. A competing writer is
refused as `checkpoint-in-use` without touching the live job. An interrupted
shell tool is reported interrupted and is never automatically replayed. SIGINT or
SIGTERM during a tool removes the whole box and exits 130 after successful
teardown. Exactly-once execution, per-command cancellation,
a scheduler and a complete `ExecutionEnv` are outside this prototype.

Its project is a disk filled from the host checkout by `from`, with
results fetched back. The host project must be disjoint from checkpoints, controller and
dependencies, Bun and Pinfold executables, PATH entries, Pinfold's host
state and ownership paths, and runtime authority. Check canonical paths,
including nonexistent paths under symlinked ancestors.
Refuse overlap as `host-boundary` before box or model work. Checkpoints
remain private on the host; a moved checkpoint namespace is refused.

Prototype-only dependencies are `@earendil-works/pi-durable`, `pi-ai` and
`chord`, with transitive `pi-telemetry`, and for development TypeScript
and `@types/node`. Its README gives launch, pinning and gate rules.

## Configuration

Layers: environment > `.pinfold.toml` > the profile's `pinfold.toml` >
built-in defaults. Each key takes the highest layer that sets it, lists
included: a list replaces the ones below it, and `allow = []` allows
nothing. An environment variable that is set, even empty, sets its key.
An override containing non-UTF-8 bytes is refused, naming only its key.
Only the environment and project select `profile`; a profile's own
`profile` value is ignored.

| Key | Env | Default | Meaning |
|---|---|---|---|
| `profile` | `PINFOLD_PROFILE` | `default` | Profile name |
| `allow` | `PINFOLD_ALLOW` | `api.anthropic.com`, `platform.claude.com`, `api.openai.com`, `auth.openai.com`, `chatgpt.com`, `openrouter.ai`, `opencode.ai`, `registry.npmjs.org`, `pi.dev` | Hosts the proxy lets through |
| `routes` | `PINFOLD_ROUTES` | `{}` | Proxy routes: `name = "host:port"` for a host service, or an injecting route `name = { to = "https://…", headers = { Authorization = { from = "VAR", prefix = "Bearer " } } }` (see Egress proxy) |
| `containerfile` | — | the profile's image | The project's Containerfile, a path relative to the project; typically FROM the profile image |
| `cpus` | `PINFOLD_CPUS` | 4 | |
| `memory` | `PINFOLD_MEMORY` | `8G` | |
| — | `PINFOLD_ENV_<NAME>` | — | `<NAME>` in the box; the only way host env enters |
| — | `PINFOLD_NO_UPDATE_CHECK` | — | `1` disables interactive update checks and notices (Updates) |

The list keys take comma-separated values in the environment:

- `PINFOLD_ALLOW`: host names; a leading `.` makes a suffix entry.
- `PINFOLD_ROUTES`: `name=host:port` pairs; an injecting route needs a file.

An unknown key in `.pinfold.toml` or a profile's `pinfold.toml` is refused,
naming the file and the key. `containerfile` in a profile's `pinfold.toml` is
refused: it is a project key.

`pinfold config` prints the effective configuration without changing state,
even when the runtime is unavailable. Its `project` names the id, `home`
and the home and project disks. `image_built` is true or false when
the runtime image inventory succeeds, and null when it cannot be observed;
`image_error` names that failure. Host configuration and trust facts remain
available. `artifacts` is also read-only.

**Trust.** `.pinfold.toml` and the Containerfile its `containerfile` names
are used only if their hashes match those `pinfold allow` recorded in
`~/.local/state/pinfold/trust` for this project. A change stops the run
until allowed again, refused as `untrusted`.

## CLI

```
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [cmd…]            bash (or cmd) in this project's running pi box
pinfold push                     this checkout's branches into its pi box as refs/remotes/host/*
pinfold fetch                    its pi box's branches into refs/remotes/pinfold/*
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]… [--no-cache]   a caller's image from its own context; one JSON line
pinfold image rm NAME            retire a caller image name; one JSON line
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE] [--builtin]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim unused disk space
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold artifacts                pinned harnesses as JSON
pinfold apparmor-profile         the AppArmor profile for this executable (Linux)
pinfold update [--check]         check for a release, or download and install it
pinfold config [ROOT]            effective configuration and project facts as JSON
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
```

`artifacts` prints one JSON array, an object per harness, then on Linux
one named `vmm`: `name`, `version`, `path` (the host directory mounted at
`/opt/pinfold/<name>`; for `vmm`, its install directory), `cached`, and
`assets`, a list of `{url, sha256}` for this host's `os-arch`. Nothing is
downloaded. `doctor` reports the same list.

`pinfold --version` (`-V`) and `pinfold --help` (`-h`) answer without
touching the runtime or the state dir; `--help` or `-h` after a subcommand, before any `--`,
prints that subcommand's syntax line and exits 0, touching neither. `pi`
passes it to pi, and `attach` to its command once one is given.

The project root is the git top level, else `$PWD`. Inside a git directory,
the command refuses as `project-in-git`.

## Guarantees

Each has one end-to-end test in the Rust E2E crate's suite, but for
three that run apart. Testing policy is in `AGENTS.md`.

- 34 is an ignored slow test in the crate:
  `cargo test -p e2e --locked --test e2e box_::blocked_route_responses_release_the_upstream -- --ignored --exact`.
- 35 is the standalone
  `examples/pi-durable/durable_recovery_preserves_boxed_execution.ts`
  gate; its README gives the commands. It runs after the Rust suite, with
  exclusive runtime ownership.
- 36 is the Linux slow gate `scripts/slow-proxy.sh`.

| # | Guarantee | Shown by |
|---|---|---|
| 1 | No network but loopback | Inside: only `lo`; `1.1.1.1` unreachable. Control: the fixture answers through a route. On Linux, from inside, a vsock connection to CID 2 on the exec port and on an unused port is reset at once, not timed out; control: CID 2 port 3128 reaches the proxy. The guest kernel's `/proc/config.gz` leaves vsock loopback unset. |
| 2 | Only allowlisted hosts get through | `api.github.com` answers. `example.com` gets a proxy 403, logged "not allowlisted"; `ready` names that log's path. |
| 3 | The proxy refuses the tricks | Each trick has a positive control: IP literal, loopback resolution, SNI mismatch, CONNECT to a route and ambiguous Content-Length framing. A raw valid Content-Length POST delivers its body to the same host fixture. |
| 4 | A route reaches exactly one host service | A route reaches the fixture with the Host header rewritten from the client's and logs method, queryless path and upstream status. Replacing its writable audit log with a directory makes the next request stop the owner with audit-log, exit 1 and, on Apple, no runtime box. |
| 5 | Nothing can gain privileges | `CapBnd` 0 and the host uid in exec'd processes; no setuid or setgid files; rootfs not writable; init's credentials are the host uid on Apple; on Linux PID 1 is root with `CapBnd` exactly `CAP_SYS_ADMIN`, the exec service is the host uid with every capability set empty and box work cannot list its `/proc/<pid>/fd`, and `unshare -U` fails. |
| 6 | The environment is exactly the spec | Unprefixed host proxy variables stay out; PINFOLD_ENV_SECRET arrives without entering host argv; on Linux it is in neither `/proc/cmdline`, the VM configuration, the init disk nor any disk image. Guest PATH, HOME, `XDG_RUNTIME_DIR`, `LD_PRELOAD`, multiline values and a transport-prefixed name arrive exactly while local list/exec/down still manage the box. Always-applied values win. |
| 7 | No egress means no way out | Without `egress`, nothing gets out, not even through a route. On Linux a vsock connection to the host proxy's port is reset; control: with a route it connects. |
| 8 | Losing the owner fails closed | After SIGKILL, egress fails closed; another XDG root sees the owner dead, prunes the box and reuses its name. An abandoned partial claim is reclaimable. |
| 9 | The lifecycle works for a caller | Without egress, `exec` immediately after `ready` streams both streams and returns the exit code, and exits 3 on an absent box; `ready` and `list` name the image's id, and `ready`'s `egress_log` is null; `list` finds the box by label; `down` removes it, and on an absent box, an empty name or a flag-like name (`--filter=…`) exits 0, prints nothing and leaves the live box alone; closing `up`'s stdin tears the box down with reason `stdin-closed`. A 60-character name comes up. On Apple, a container pinfold did not create is absent to `list`, `exec`, `stat` and `down`. A SIGSTOP owner keeps its claim after down times out; a second XDG root waits the ten-second claim deadline before name-in-use and cannot replace it until acknowledged teardown. |
| 10 | Work leaves the box only by fetch | A box writes, chmods and deletes on its disk; the host project's bytes, modes and mtimes are unchanged. Before `fetch`, the box plants an executable hook under every client-side hook name, `core.hooksPath` and `core.fsmonitor` in its config, and tracked `.githooks/` and `.husky/` files, each touching a host marker path. `fetch` lands its branches under `refs/remotes/<box>/*` and no tag; after a fast-forward merge and two checkouts on the host, no marker exists. Control: the same hook placed in the host's `.git/hooks` creates the marker. A box that emits its own bundle fails the fetch and changes no host ref and no path or size under the host's `.git/objects`, for each of six bundles: one with a `refs/heads/x:refs/heads/injected` head, one with a `refs/heads/x:refs/replace/*` head, one whose `refs/heads/x` commit holds a `.git` tree entry, which fails fsck as `hasDotgit` on any host git, one whose `refs/heads/x` names a commit its pack lacks, one with both `refs/heads/x` and `refs/heads/x/y`, and a sound bundle beside a listed `refs/heads/x` it leaves out, at a host commit the box's namespace does not reach. |
| 11 | Disks persist and attach exclusively | A disk created with `from` holds the host tree's files with their exec bits and a symlink as a link, and no FIFO; files the host adds after that never appear; after its owner is SIGKILLed during the copy and its box pruned, `pinfold fetch` in the project whose disk it is exits nonzero, and the next `up` naming the disk holds the whole tree. A `from` whose sizes (one sparse file) pass the disk's `size` is refused as `limit` with no disk left. A file one box writes to a disk is read by the next box that attaches it. On Linux, a nested disk whose mount point is a symlink the box planted on its disk ends the start `failed`, naming it, and leaves nothing at the link's target; the same nesting at a plain path mounts writable. While a box attaches a disk writable, a second `up` naming it is refused as `disk-in-use` and creates nothing, and of two writable `up`s racing for one new disk exactly one runs and the other is refused as `disk-in-use`; after `down`, two boxes attach it read-only at once and neither can write it, nor any mount. After the runtime kills a writer's box and its owner exits, and after a writer's owner is SIGKILLed and its box has stopped, before any `prune`, a read-only attachment is refused as `disk-unclean`; a writable one runs, and after its `down` a read-only one runs. On macOS, while the runtime lists a box whose owner is dead as created but never started, a writable `up` naming its disk is refused as `disk-in-use`. While one state root attaches a disk writable, a second root's writable `up` naming it runs, does not see the first root's files, and its `disk list` names only its own box. `disk rm` of an attached disk is refused as `disk-in-use`; after `down` it removes the disk, an absent disk exits 0, and a malformed name is refused as `spec`. A write past a `1G` disk's size fails in the box with ENOSPC. |
| 12 | A changed project file stops the run | A domain added to `.pinfold.toml` by a box commit, fetched and merged on the host, or a changed project Containerfile, makes the next run, `pinfold fetch` and `pinfold build` refuse as `untrusted` until `pinfold allow`. With no identity in the repository or home, the box's commits arrive authored `pi <pi@pinfold.invalid>`. A start pushes host commits in: a clean box branch strictly behind is fast-forwarded to the host's commit; a dirty tracked file or a local box commit leaves the box's HEAD where it was while `host/main` still moves; an untracked box file the fast-forward would overwrite prints `sync-failed` and pi still runs. |
| 13 | Project state persists and stays separate | Settings survive reseeding. Guest writes in two same-named projects remain separate on A/B/A runs. A second `pinfold pi` in a project with a live box is refused as `disk-in-use`. A long valid project basename launches. |
| 14 | Both pi config levels load behind a route | A routed fake model receives profile and project skills and the host's allowlist. |
| 15 | Cleanup removes only pinfold's garbage | After three builds of one source, two images remain; an image a box still uses survives later builds; a source a build left above two drops to two on a later `pinfold clean` with no build after, and of three build tags sharing one image it removes only the oldest. `--dry-run --unused 0s` leaves a project state the real one removes. A project whose checkout was moved keeps its state and disks under plain `clean`. An unlabeled image, a live box, its project's state and disks survive `pinfold clean`, also with `--unused 0s`, which removes an idle project's state and disks; a dead box is removed and protects nothing. On Apple, a container started with the runtime's CLI from a pinfold-built image survives both, and a build in another state directory held active in RUN succeeds across clean. `image rm` of a name removes its tags whatever their age and no other name's, and reports `removed` with the image it kept `in_use`; an image's last tag stays while a box uses it. Two names built from identical inputs share one image id; `image rm` of one removes its tags while a box of the other name runs the image, and leaves the other name's tags, the image and the box. On Linux, the image's root disk goes with it and stays while a box's image holds it. A claimed pi startup awaiting its real harness download keeps its project's state across `clean --unused 0s` while its checkout is absent; the launch then succeeds, and the same state is removable afterward. |
| 16 | The highest layer sets the allowlist | The environment replaces project allow, CPU, memory and route settings. Kernel observations and a host HTTP fixture prove the result. Invalid UTF-8 overrides fail without disclosing their bytes. |
| 17 | up refuses before it creates | A missing image, a misspelled nested spec key, a bad env name, a `dev.pinfold.` label, a suffix allow entry on or above a public suffix, a mount or disk path with a comma, a mount or disk at or below `/proc`, `/sys` or `/dev` (also through `..`), a disk at `/`, `/tmp`, `/etc` or `/etc/resolv.conf` (also through `..`), a disk name that is a flag or a path, and a live name are refused as data, naming the cause; the misspelled key's refusal also names the box; each leaves no box and no state dir. Of two `up`s racing for one name exactly one wins, and the winner answers exec. |
| 18 | A caller reads the effective configuration as data | config reports project policy, trust, a built image as built, and the home disk actually attached. Without a runtime it still reports host policy, image_built null and image_error without creating state/cache/config files. Unknown project/profile TOML keys and a profile's containerfile are refused with the source and key; correcting the same file restores the fixture policy. With `PINFOLD_PROFILE=full` and no project `allow`, config reports the default list plus `github.com` and `api.github.com`. |
| 19 | A caller-owned box launches the pinned harness | A spec with each harness (pi, claude, codex) runs `/opt/pinfold/<name>/<name> --version` at the version `pinfold artifacts` pins, and the box's PINFOLD_ALLOW is the spec's allow list. After the host deletes the cached pi executable, a new pi box repairs it and runs the pinned pi. |
| 20 | A caller can tell an OOM kill from a failure | On Linux, the guest kernel's `MemTotal` is within 64 MiB under the box's memory, and a command that exceeds it is killed and stat's oom_kills rises; on both runtimes stat reports the limits in force, every field is present, and `exec`'d processes carry `oom_score_adj` 1000, so init is never the victim. |
| 21 | An injecting route keeps the credential on the host | The fixture behind an injecting route receives the header; the box's environment and the egress log never hold the value; an https route reaches api.github.com over TLS. |
| 23 | A caller builds an image from its own tree | An image built from a caller's context with a COPYed file reaches a box as that file; the built line carries the unique ref, the image's id and the labels; three builds of one name move latest, and the first build's ref still comes up, on Linux with the build's labels in `ready`; two builds of unchanged inputs make one image under two refs; a failed build prints a bounded trailing log even after a 200,000-byte unterminated line, retains its final context marker and makes no image. Cached caller builds preserve a RUN-generated random stamp; --no-cache changes it. On Apple, builds from two state directories with different `NO_COLOR` and `BUILDKIT_COLORS` both succeed while one is held active in RUN. |
| 24 | Every build reruns its steps | A second build changes a random RUN-generated stamp. On Podman the fixture-labeled set of dangling image IDs gains no member; concurrent builds and deletion of old images cannot conceal or fabricate a leak. |
| 25 | `--version` needs no runtime | `pinfold --version` prints the version, and `pinfold box list --help` exits 0, with no runtime and leaving the state dir untouched. |
| 26 | A login route keeps the login on the host | With `to` at a host fixture, claude: the fixture receives the `from` token as a Bearer header and the box has `ANTHROPIC_BASE_URL` and a placeholder. codex, with `CODEX_HOME` holding a login whose token lapses within 5 minutes and `CODEX_REFRESH_TOKEN_URL_OVERRIDE` at a fixture: the model fixture receives the refreshed token and its account header; the box's environment, files and the egress log hold neither token. With that refresh held, a caller under distinct XDG roots and the same CODEX_HOME reports `login-busy` once; no second refresh reaches the fixture, and releasing the first lets both boxes use the refreshed login. A stalled real helper refresh is deadline-bounded; cancellation reaps the helper and closes its fixture connection before returning without a box. An empty `CODEX_HOME` is refused as `login`. On macOS only, with a real login, codex completes one tool round trip through a route with no `to`. |
| 27 | Doctor reports without changing state | With and without a runtime, doctor leaves complete Pinfold-owned state/config/cache trees unchanged, including absent roots, and reports host configuration. Native runtime bookkeeping is outside those trees. Linux runtime-dir and missing-kvm diagnostics use real failures. The Linux host keeps AppArmor's unprivileged user-namespace restriction on (`kernel.apparmor_restrict_unprivileged_userns` is 1); there a copy of pinfold outside its profile's path reports `userns-apparmor` and is refused as `runtime`; the profiled executable starts a box. |
| 28 | Host updates are verified and atomic | A host HTTPS release fixture supplies a different valid executable. Wrong checksums and live owners preserve the original bytes/inode; a working command started during a held asset transfer also prevents replacement. After teardown the verified bytes replace the inode and preserve the invoked command symlink. |
| 29 | Interactive update checks are bounded | A host HTTPS fixture observes one interactive check per interval and none for opt-out/noninteractive commands. A stalled request closes within 1.75 seconds of fixture receipt and is not retried by the next launch. |
| 33 | Projects exclude host authority | `allow`, build and pi refuse state, config or cache roots inside the project, including nonexistent paths reached through a symlinked ancestor. External roots allow the same project to run. |
| 34 | Blocked route responses release the upstream | A normal routed response succeeds. A guest that keeps its upload open but stops reading causes the host fixture connection to close after the production write deadline, while the guest holder remains alive. Slow gate only. |
| 35 | Durable recovery preserves boxed execution | The host fixture refuses authority overlap before work, then, with the project on a disk, observes a successful boxed command whose commit is fetched back as a host ref, a competing checkpoint writer refused without disrupting its owner, a mutation followed by SIGKILL before tool-result commit, a new box generation before recovery, no unsafe replay, and whole-box cancellation including a background child. Standalone prototype gate on macOS and Linux x64. |
| 36 | CONNECT tunnels share activity | Initially idle TLS tunnels start at staggered times and expire after the production idle interval. Real TLS tunnels carry upload-only and download-only traffic for longer than the production idle deadline. Neither closes while bytes flow; both then close after a full idle interval with `idle timeout` in the egress log while the guest holders remain alive. Standalone Linux slow gate. |
| 37 | Each box has its own kernel | Inside, `uname -r` and `/proc/version` differ from the host's, on both runtimes. On Linux `uname -r` is the guest kernel `vmm.toml` pins. |
| 38 | Each run's history is its own | Two `up`s of one name in sequence; the first makes an allowed and a refused request before `down`. The second's `ready.run_id` equals its `dev.pinfold.generation` label and differs from the first's, and its `ready.egress_log` is empty before its first request. After the second makes its own request, `events` for the first run still returns exactly the first run's two decisions. Each `down` names its own run_id. |
| 39 | The launch record is the applied policy | A pi box starts from a trusted `.pinfold.toml` with an allow list, a host-service route and an injecting route whose header comes from a host variable, plus `PINFOLD_ENV_SECRET`. While it runs, the file's allow list changes and is not re-allowed: `config` reports the change, but `inspect` still reports the started allow list, route kinds and targets, cpus and memory, equal to what the test wrote, plus env names that include `SECRET`; the box's next request is decided by the started list. No file under the state root holds the secret or the header value. |
| 40 | A run's end is recorded apart from its start | When `box down` returns, `inspect` already reports terminal `signal`. An init killed from the host runtime on Apple, and a VMM SIGKILLed from the host on Linux, gives terminal `exited` whose detail names exit status 137. An owner SIGKILLed after `ready` leaves the launch record `available` and terminal `not-observed`, also after `box prune` removes its box. A pi run whose harness exits records terminal `harness-exited`. |
| 41 | Run queries are bounded and need no runtime | With no runtime on `PATH`, `inspect` and `events` answer for a finished run. A run with 150 decisions, paged at the default limit while its box keeps appending, returns every line once, in order, across `next`. A no-egress run reports egress `not-applicable`. A malformed run id, `--limit 101` and another run's cursor exit 1; an unknown run id exits 3. |
| 42 | Run records are retained within bounds | `clean` removes an inactive run whose directory is aged 15 days on the host (`inspect` exits 3), while a live run aged the same stays `available` and its log keeps appending. With inactive runs padded on the host past 128 MiB in valid decision lines, `clean` removes the oldest inactive run and keeps the newest. |
| 43 | The VMM reaches only its own box | Linux only. With two boxes up, each VMM is a child of its `box up` and runs as the user with `CapBnd` 0 and `NoNewPrivs` 1; its user, mount, PID, network, IPC and UTS namespaces differ from `box up`'s; its root, read through `/proc/PID/root`, holds exactly its jail's list and not the other box's disks; none of its mount points is under the user's home; its fds 0–2 are `/dev/null` and none is under the home; SIGKILL of `box up` ends the VM. |

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
and the TLS client for injecting routes: `rustls` (ring) with
`rustls-native-certs` for the host's roots.
SNI comes from a small ClientHello parser. The Public Suffix List is
data, `crates/pinfold/public_suffix_list.dat`, embedded with `include_str!`.

- Platform dirs are the literal XDG-style paths on both OSes:
  `~/.config/pinfold`, `~/.local/state/pinfold` and `~/.cache/pinfold`,
  with `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` honored.

## Open questions

- `198.18/15` blocking breaks fake-IP DNS proxies (Surge, Clash).
  Configurable, or documented.
- User namespaces in the Apple guest kernel: can they be turned off
  without giving box work a capability?
- Linux arm64 returns once a KVM host can run its suite; GitHub's arm64
  runners have none.
- Where herdr reads `HERDR_AGENT`.
- Freed blocks inside a disk: whether they return to the host, or a
  disk's allocation only grows.
