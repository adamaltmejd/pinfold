# Linux project configuration cases and builder diagnosis (#294), 2026-11-14

The original failure came from the trial's dynamically linked GNU debug
binary. The Linux CLI is also the guest init. Its init disk holds only
`pinfold`, `dev` and `mnt`, without the binary's dynamic loader. Building
the unchanged source for static musl fixed the build and the box cases.
No host kernel, KVM, memory, permission or pinfold source change was needed.

The leased VM ran Debian 13 amd64, kernel
`6.12.107+deb13-cloud-amd64`, with 2 vCPUs, 3922 MiB RAM, no swap and
nested KVM. The source was commit
`7379a7334f166388f8c314beaff320da62878c1c`; its `crates/` content matches
this branch's starting HEAD, `8aa6948`. The original report named the
earlier implementation baseline `2a9bf28`. The build and environment
output is in `2026-11-14-project-config-linux/build-output.txt`.
All output paths and user identities are redacted. Line endings and trailing
whitespace are normalized. No credentials were used.

## Cause

The original case output is retained as `linux-output-before.txt` in the
directory beside this record. Its configured-project command was:

```text
$ pinfold build
builder exited before handshakepinfold build: the BuildKit build failed
[exit 1]
```

The standalone diagnostic was run on the VM from `/home/user/pinfold`:

```sh
bash /tmp/pfh-diagnose.sh target/debug/pinfold
```

`diagnose-start.sh` is that script. `diagnose-output.txt` records its
configuration and full console output. It used the pinned Firecracker
1.17.0, pinned guest kernel 6.18.57, pinned read-only builder root,
`run_job`'s boot arguments, 2 vCPUs, 2560 MiB and the same drive roles
and vsock CID. It ran Firecracker directly with an empty environment and
console output, outside pinfold's jail. Its private work, input and selected
disks were small empty filesystems; no real build job was sent. This
diagnostic establishes the failure before init, not builder security or
a completed build. Its temporary disks were removed on exit.

The decisive output was:

```text
$ file target/debug/pinfold
ELF 64-bit ... dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2
$ pinfold doctor
  preflight: ok
Run /pinfold as init process
Kernel panic - not syncing: Requested init /pinfold failed (error -2).
Firecracker exiting successfully. exit_code=0
```

The init executable exists; its requested ELF interpreter does not.
The kernel's ENOENT is consistent with that missing loader. The successful
musl run below confirms the diagnosis on the same VM. Firecracker booted
both vCPUs and the 2560 MiB guest. Increasing VM memory or weakening the
jail was unnecessary.

## Correction and cases

The installed Rust musl target and `musl-gcc` were already available.
The detached VM build used:

```sh
cd /home/user/pinfold
RUSTC_WRAPPER= cargo build --locked --target x86_64-unknown-linux-musl
```

It exited 0. `file target/x86_64-unknown-linux-musl/debug/pinfold` reported
`static-pie linked`; `readelf -l` reported no interpreter.
The original GNU debug binary was preserved.

The case script was then run as a detached job and polled through
`labvm exec -- bash -lc '...'`:

```sh
bash /tmp/pfh-cases.sh target/x86_64-unknown-linux-musl/debug/pinfold
```

`linux-cases.sh` is that script. `linux-output.txt` replaces the earlier
failed output. The script exited 0. Each case records its command and
individual exit status; the script's final status alone is not a pass.

The script now creates the required maintenance stamp before its first
pinfold command. It uses isolated short XDG config/state roots and the real
cache. It copies the built-in default profile to `probe-linux294`, so every
new image has a probe name. It sets `PINFOLD_CPUS=2` and
`PINFOLD_MEMORY=1G` for boxes rather than using the project's 8G default
on this 4 GB VM. These variables do not change the builder's fixed size.
It checks for at least 12 GiB free before building; 24 GiB was available.

| Case | Command and observed output | What it establishes |
|---|---|---|
| D, outside Git | `pi --version`, `config edit`, `config`, `build`: `not-git`, exit 1. Two state entries, the root and maintenance stamp. | No project configuration or state is created. |
| A, no configuration | `pi --version`: `no-config`, exit 1; zero project-state directories. | A Git project requires host configuration. |
| B, configured Git project | `config edit`: exit 0. `build`: probe image ref, exit 0. `pi --version`: `1.5.2`, exit 0. `push` and `fetch`: JSON success, exit 0. | A real box starts with the configured project. `push` and `fetch` succeed on unchanged history; a transfer of new commits is not shown. |
| E, other mounts | Change mounts to an `/etc` share, then `pi --version`: `spec`, exit 1. | The tested non-template mount is refused. |
| C, cache inside project | Set a symlinked cache path into the project, then `pi --version`: `host-path-in-project`, exit 1; zero entries written there. Restore the external root: `1.5.2`, exit 0. | The host-path refusal and its positive control both work. |

## Findings for the owner

1. **Preflight misses an unusable Linux init.** The GNU binary reports
   `preflight: ok` although it cannot boot its own init disk. The spec's
   Code section requires a static musl Linux binary. Proposed change:
   shared Linux preflight inspects its own executable's ELF program
   headers and rejects `PT_INTERP` as `init-dynamic`, naming the interpreter
   and `cargo build --locked --target x86_64-unknown-linux-musl` as the
   correction. Doctor reports the same finding; builds and box starts
   refuse before preparing disks. This diagnoses the observed failure
   without booting a probe or changing VMM confinement. Add the named
   requirement to ARCHITECTURE.md in the implementation commit. No such
   code or spec change was made here.
2. **The handshake error drops the child status.** In
   `crates/pinfold/src/core/image/linux/job.rs`, `run_job` discards the
   result of `child.wait()` and reports only `builder exited before
   handshake`. Proposed change: include the returned exit status or wait
   error. Here Firecracker exits 0 after the guest panics, so status alone
   cannot name this cause; the init preflight check above is needed.
   Firecracker console capture would change the exact `/dev/null` fd
   contract in guarantee 43 and needs a separate design decision. It is
   not proposed as an incidental logging fix.

## Cleanup, checks and limits

The case script removed its home and project disks with
`pinfold disk rm --all`, then removed its scratch root. The manual
diagnostic removed its scratch disks. `cleanup-output.txt` records an
empty probe box list and no Firecracker processes after the run.
The existing lease remains active, as requested. It was neither acquired
again nor released.

One completed image remains on the leased VM, manifest
`sha256:7a76df9a1bf093743a7486ef023bd32fd60f456102addd10c6a9fbb6a80bf761`,
under both refs:

```text
localhost/pinfold/profile-probe-linux294:18e7fcefb8b86e97-25846
localhost/pinfold/profile-probe-linux294:latest
```

The static debug build and normal shared build/artifact caches remain.
This run's detached-job scripts and copied logs under `/tmp` were removed.
No unrelated image, box, disk or previous scratch directory was removed.

tests, release, push or PR were made. The suspended suite and macOS runtime
were not run. These are the listed Linux #294 cases, not full guarantee
coverage or a security audit.
