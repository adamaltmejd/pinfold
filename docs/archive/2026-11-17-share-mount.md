# Apple shares with protected Git metadata

Initial implementation: `d9898f8`. Review fixes: `05ab5d2`.
Implemented 2026-11-17 from `a9e1ec68a76a2e2f854dd8cb82ad296f57cd0701`,
for #279. The suite remains suspended. These are real-binary observations,
not a replacement end-to-end suite. No test or dependency was added.
Outputs redact home and fixture paths and trim trailing whitespace.

## Finding and operator ruling

The initial finding is retained in commit `31f2e7a` and
[native output](2026-11-17-share-mount/native.txt). The default image's
`/var/run -> /run` makes distinct declared destinations alias. A writable
attachment at the alias of `.git` changed what the box saw there, while
the protected host Git bytes remained unchanged.

FIXES-1 accepted that observation and declined an image inspection phase.
The control is protection of the admitted host Git metadata. A caller's
separately declared source may replace the visible source through an image
alias. Generated Git protection follows every caller attachment. Lexical
conflicts and host-source exposure of protected Git refuse before resources.
The supplied init attaches last, read-only. No image parser, image-tree
reader or temporary image-inspection runtime was built.

Before implementation, `ruling-probe.py` (retained in `31d23ec`) exercised image aliases of the
share root, `.git`, and a parent, with the caller before and after the
share and generated Git last. All six native cases retained the protected
host bytes. The `.git` alias wrote only the replacement fixture. The
reason is that guest path resolution selects among declared sources; it
cannot add a host source. Source and inode admission exclude other writable
exposures of the protected files. The guest privilege controls keep its
read-only attachment in place. This reasoning excludes host mutation after
admission and guest-kernel compromise, as the spec states.

## Implementation and inspected controls

Core mounts accept `readonly` (the default) and Apple-only `share`.
Admission checks one resolved set of protected paths in both directions,
including external artifacts, init and selected profile directories. Native
ancestor identities cover alternate-case source spellings. Nested share
destinations compare opened component identities against protected Git;
case variants on case-insensitive sources therefore refuse before mounting.
Shares use plain source directories and literal Git ownership destinations.
Readonly-only and disk-only plans receive no share checks. Overlapping share
sources refuse only when a protected root Git directory is exposed writable.
Project validation reuses core's protected-path check with its own file paths.

A descriptor-relative no-follow walk collects multiply linked protected Git
file identities, rejects protected symlinks, and checks outside files in all
shares against that set. Ordinary hardlinks remain allowed. Generated Git
mounts carry a private marker; callers cannot request that marker. Apple
emits caller attachments and disks, then Git protection, then supplied init.
Each recorded mount has its applied kind, including generated Git protection.
Exact guest-root Git ownership exceptions preserve declared Git configuration;
the generated GIT_CONFIG variables are always-applied exceptions. Launch env
lists declared variable names, not all runtime-generated variables.

Linux's platform check precedes maintenance, init extraction, credentials and
claims. Public startup also defers maintenance and extraction until admission
finishes. Public exec and fetch return 3 until startup completes. Their
generation-checked dispatch locks prevent name reuse during a command. The
owner publishes ready only after root setup, disk filling and launch record.
Project commands already wait for core startup, initial synchronization and
`ended_if_ready` before `Reply::Admitted` in `pi/session.rs`; no new project
gate was needed. Signal waiting on maintenance remains a follow-up.

Inspection: `make_proxy_connectable` executes the supplied static init by
absolute path, with root cwd `/`, and only `init proxy-socket`. It executes no
image program and performs no environment lookup. Init reads kernel mount
information before opening the fixed socket path, excluding writable share
attachments redirected there. Apple's socket itself is a native tmpfs bind.
Descriptor-relative no-follow opens pin the root-owned socket. Permissions
are set through its `/proc/self/fd/N` link; the raw syscall and number are
removed. Ancestor rejection uses writable virtiofs mount flags, so readonly
ancestors and disks retain their behavior. A readonly `/var` with egress is
exercised in this round. Disk preparation still attaches no shares. Share
admission runs as the host uid; ordinary init, exec and session helpers retain the
existing host uid and privilege restrictions. Session helper behavior was
inspected; project share integration belongs to #280.

Removed the image's root `chmod` execution and duplicated project-only
protected-path implementation. README and ARCHITECTURE state the implemented
contract and image-alias limit. Project defaults, project-owner lifecycle,
project crossing rules, agent instructions and profile files were not changed.
Public core fetch now shares the startup dispatch gate.

## Apple public CLI observations

Command:

```sh
PF279_IMAGE=localhost/pinfold/image-probe-share279-env:latest \
  python3 docs/archive/2026-11-17-share-mount/accept.py
```

The earlier `accept.py` is retained in `31d23ec`.
[Apple output](2026-11-17-share-mount/apple.txt) retains the commands,
exit statuses and observations. Refusal labels cover both caller orders,
nested and parent destination conflicts, protected and alternate-case source
aliases, overlapping writable sources, symlinked destinations, Git hardlinks,
protected symlinks, top-level Git files/symlinks, cross-share hardlinks,
protected roots, selected profile sources and normalized init overlap.
They refuse as `spec`, with no box or run state created. Corrected trees run.

The admitted share includes an outside hardlink and 256 hardlinks to a copy
of the actual built binary under `target/`. This is a hardlink fixture, not
a measured walk of a built Cargo target tree. Ordinary edits reach the host;
ordinary Git reads succeed with the exact `/project` ownership exception.
The caller's `core.abbrev=11` remains effective. Launch records show readonly,
share and generated readonly Git attachments.

Direct append, truncate, unlink, file rename/replacement and Git directory
rename/replacement/removal all fail. Alternate-case `.GIT` writes fail too.
After every attempt, the retained protected snapshot is unchanged. It covers
path names, bytes, modes, device/inode, link count, size, uid/gid and mtime/ctime;
it deliberately excludes read-induced atime. Remount, unmount, mount-over and
user-namespace attempts fail for concrete privilege reasons. Successful
ordinary writes and Git reads run in the same box. An outward symlink cannot
read the unmounted host fixture. Guest uid/gid match the recorded host values;
CapBnd is zero, NoNewPrivs is one and the user namespace limit is zero.

The earlier host commit produced six failures in eleven concurrent reads;
a following Git log reported no commits and returned 128. That was an
internally inconsistent view, and no retained line established recovery.
Calling it only stale reads was incorrect. The fix-round observation below
adds a later status/log checkpoint. No cache, wrapper, index copy or
synchronization delay was added.

A public image-alias box changes the replacement source and leaves protected
host metadata unchanged. A non-Git share edits host files without generating
Git protection. Existing readonly mounts and readonly profile files retain
their write refusals. A fresh home disk receives the existing profile seeds.
Every created box is down and absent to public stat; the home disk is removed.

The hostile-path image changes only default-image configuration:

```dockerfile
FROM localhost/pinfold/profile-default:latest
ENV PATH=/project/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /project
```

I built that probe image in the prior round, without first stating the
image build in commentary. The command was:

```sh
XDG_CONFIG_HOME=/tmp/pf279-image/config XDG_STATE_HOME=/tmp/pf279-image/state \
PINFOLD_NO_UPDATE_CHECK=1 target/debug/pinfold image build probe-share279-env \
  --containerfile /tmp/pf279-image/Containerfile --context /tmp/pf279-image/context
```

It used an empty context and that Containerfile; no profile rebuild. No
image was built in the fix round; it reused the retained image.
The share's executable `bin/chmod`
would write a uid marker. With that image PATH and cwd, public startup
succeeds, the socket is mode 666 and the marker is absent. The actual fixed
permission operation is identified in the inspection above.

## Linux and checks

The runtime observations in `linux.txt` are from the initial implementation,
not a Linux runtime rerun for FIXES-2. This fix round reran Linux-target
clippy on the final source; its command and exit status are in `build.txt`.

The specified development host did not resolve. The approved fallback used
the homelab-trial-vm skill and an exclusive disposable Debian 13 lease.
Only public source plus this patch and fixture scripts were transferred.
The guest user received KVM group membership inside that disposable VM.

[Linux output](2026-11-17-share-mount/linux.txt) records the real musl
binary's recognized-share `spec` refusal with runtime PATH unavailable,
maintenance due and all four inspected roots absent before and after.
Repeats retain absent XDG roots and leave the existing ownership directory
present. On the same host, readonly fixture reads succeed and writes fail
read-only; a writable disk retains its bytes across two boxes. Both owners
end successfully, the disk is removed, the box inventory is empty and the
fixture is removed. `linux.py` retains the exact commands and tiny recipe.

`cargo build --locked`, `cargo fmt --check` and
`cargo clippy --all-targets --locked -- -D warnings` pass on the Mac.
Linux build and clippy pass with `--target x86_64-unknown-linux-musl`.
[Build output](2026-11-17-share-mount/build.txt) retains disk checks,
image identity, host uid/gid and successful check output. Every Mac build's
recorded free disk exceeds 12 GiB. No release was cut.

## FIXES-2: PR #328 fix round

All three reviews were read. `fixes.txt` retains before/after CLI results;
`before.py`, `startup.py` and `fixes.py` retain the corresponding fixture
commands. Before the fix, existing `.GIT/hooks` passed admission. A missing
case-variant target passed admission then failed read-only at mount time;
protected bytes and names stayed unchanged in that observation. Both now
refuse as `spec` before resources, with unchanged protected metadata.

Before the startup gate, real disk preparation was observable through the
native inventory, and public exec of `id -u` returned 0 with uid 0. The same
window now returns 3 for exec and fetch; after ready, exec reports the host
uid and fetch transfers a fixture commit. No test flag or runtime stub was
used. The fixture is a 512 MiB sparse zero file on a disposable disk.

The fix-round CLI rejects descendants of externally linked artifacts,
profile directories and profile share directories. Trailing-star Git trust
patterns refuse, while an embedded literal star works with ordinary Git.
Readonly nested symlink destinations, readonly and disk ancestors of init,
a disk populated from Git data, and readonly `/var` with egress run.
Overlapping non-Git shares, overlap outside protected Git, and two views of
one protected root run. An ancestor exposing nested protected Git refuses.
Case-variant Git rename, rmdir and file writes fail without metadata changes.

The fixed root operation produces socket mode 666 with no marker. The old
image-program lookup positive control runs the fixture chmod and records
uid 0. Its first version also tried to chmod a user-owned harmless file,
returned 1 after writing the marker, and stopped the driver. The corrected
marker-only control returns 0; production code did not change for that fix.

One host-commit case ran. Six of ten concurrent Git log reads failed.
Immediately afterward status returned 0 while log returned 128/no commits.
At the fifth observation, 1.071 seconds after host commit completion, both
returned 0 and log matched the new host commit. Host Git metadata remained
unchanged by guest activity. This establishes recovery in that observation,
not a fixed recovery bound. The polling interval includes public exec time.

Every requested deletion was done; none was skipped. This removes the extra
parser platform check, speculative share preparation in project validation,
duplicate generated-path checks and disk-from-Git rejection, duplicate init
embedding/extraction, duplicate directory-stream wrapper, duplicated reverse
protected-path list and raw chmod syscall. One shared directory visitor and
one startup extraction remain. Live project crossing retains its cache
repair call; new transient starts prepare init once in core. Project-specific
error behavior is preserved.

The spec now limits protection to root Git and boxes sharing exactly that
root, excludes overlapping other boxes and permitted external file aliases,
and names generated Git environment and maintenance timing. It does not
build cross-box coordination or an image inspection phase. The approved Git
symlink refusal remains, including symlinked hooks. The named five deferred
items are listed at the end; no ticket was created here.

`05ab5d2` resolves FIXES-2 items 1-7 and the spec corrections: Sol's
protected-profile, root-invocation and Git-trust findings; Astra 1/3 and
overlap; Claude B1/B2/B4/B5/F5. The cross-box findings (Sol, Astra 2,
Claude B4) are resolved by the operator-approved limit, not coordination.
This record commit closes FIXES-2 item 8 / Claude B3 and F7 with observed
recovery, the positive control, exact check commands and image-build account.

## Caller changes and cleanup

Caller changes: core mount `kind`, the Apple writable share, automatic
readonly Git protection and exact Git ownership exception; new share/source,
protected-path and layout `spec` refusals; early Linux share refusal;
per-mount kind and generated Git record. Readonly-only records retain their
declared paths. Public exec/fetch return 3 before completed startup.
An owner started by an earlier binary publishes no readiness, so exec and fetch against it exit 3 until it is restarted.
Box-up parsing/admission precedes its maintenance and init extraction.
No CLI flag or exit code changed. Project configuration still accepts disks.

The [environment record](2026-11-17-share-mount/environment.txt) retains final
inventory, fixture cleanup, VM release and free disk. Only the existing
buildkit remains running on the Mac; no volume remains. The probe image
`localhost/pinfold/image-probe-share279-env:latest` is retained as instructed.
The Linux leases are released and erased. Supplied BRIEF, FIXES-1, FIXES-2,
PLAN, REFERENCE and REVIEWS files remain uncommitted. Nothing was pushed or published.

## Open follow-ups

- Measure share walk cost before choosing any optimization.
- Bound directory depth or descriptor use in the share walk.
- Add entry paths to currently pathless walk errors.
- Make signal waiting on maintenance responsive.
- Validate malformed GIT_CONFIG_COUNT before claims/preparation.
