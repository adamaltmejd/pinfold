# Apple shares with protected Git metadata

Implementation commit: `d9898f8`.
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

Before implementation, `ruling-probe.py` exercised image aliases of the
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
Admission checks canonical host paths and native ancestor inode identities,
including alternate-case source spellings. Shares use plain source directories.
It checks protected roots and selected profile paths, source and guest
conflicts, and nested host destination components without following symlinks.
Project validation reuses core's protected-path check with its own file paths.

A descriptor-relative no-follow walk collects multiply linked protected Git
file identities, rejects protected symlinks, and checks outside files in all
shares against that set. Ordinary hardlinks remain allowed. Generated Git
mounts carry a private marker; callers cannot request that marker. Apple
emits caller attachments and disks, then Git protection, then supplied init.
Each recorded mount has its applied kind, including generated Git protection.
Exact guest-root Git ownership exceptions preserve declared Git configuration.

Linux's platform check precedes maintenance, init extraction, credentials and
claims. Public startup also defers maintenance and extraction until share
admission finishes. The admission worker retains signal cancellation.

Inspection: `make_proxy_connectable` executes the supplied static init by
absolute path, with root cwd `/`, and only `init proxy-socket`. It executes no
image program and performs no environment lookup. Init reads kernel mount
information before opening the fixed socket path, excluding share/disk
attachments redirected there. Apple's socket itself is a native tmpfs bind.
Descriptor-relative no-follow opens pin the root-owned socket; syscall
`fchmodat2(AT_EMPTY_PATH)` changes that inode's mode only. The pinned libc
omits the arm64-musl syscall constant; both supported guest architectures
use syscall 452. Disk preparation still attaches no shares. Share admission
runs as the host uid; ordinary init, exec and session helpers retain the
existing host uid and privilege restrictions. Session helper behavior was
inspected; project share integration belongs to #280.

Removed the image's root `chmod` execution and duplicated project-only
protected-path implementation. README and ARCHITECTURE state the implemented
contract and image-alias limit. Project defaults, owner integration, crossing,
agent instructions and profile files were not changed.

## Apple public CLI observations

Command:

```sh
PF279_IMAGE=localhost/pinfold/image-probe-share279-env:latest \
  python3 docs/archive/2026-11-17-share-mount/accept.py
```

[Apple output](2026-11-17-share-mount/apple.txt) retains the commands,
exit statuses and observations. Refusal labels cover both caller orders,
nested and parent destination conflicts, protected and alternate-case source
aliases, overlapping writable sources, symlinked destinations, Git hardlinks,
protected symlinks, top-level Git files/symlinks, cross-share hardlinks,
protected roots, selected profile sources and normalized init overlap.
They refuse as `spec`, with no box or run state created. Corrected trees run.

The admitted share includes an outside hardlink and 256 hardlinks to a copy
of the actual built binary under `target/`. Ordinary edits reach the host;
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

The host commits while the guest reads Git and edits ordinary files. In the
final run, six of eleven concurrent Git reads failed; a following Git log
also returned 128. The host commit succeeds and protected metadata remains
unchanged by guest work. This is the documented stale-read limit, not a
claim of continuous Git-read availability or a measured visibility bound.
No cache, wrapper, index copy or synchronization delay was added.

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

Built with public `image build probe-share279-env`, an empty context and
that Containerfile; no profile rebuild. The share's executable `bin/chmod`
would write a uid marker. With that image PATH and cwd, public startup
succeeds, the socket is mode 666 and the marker is absent. The actual fixed
permission operation is identified in the inspection above.

## Linux and checks

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

## Caller changes and cleanup

Caller changes: core mount `kind`, the Apple writable share, automatic
readonly Git protection and exact Git ownership exception; new share/source,
protected-path and layout `spec` refusals; early Linux share refusal;
normalized recorded paths, per-mount kind and generated Git record.
Box-up parsing/admission precedes its maintenance and init extraction.
No CLI flag or exit code changed. Project configuration still accepts disks.

The [environment record](2026-11-17-share-mount/environment.txt) retains final
inventory, fixture cleanup, VM release and free disk. Only the existing
buildkit remains running on the Mac; no volume remains. The probe image
`localhost/pinfold/image-probe-share279-env:latest` is retained as instructed.
The Linux lease is released and erased. Supplied BRIEF, FIXES-1, PLAN and
REFERENCE files remain uncommitted. Nothing was pushed or published.
