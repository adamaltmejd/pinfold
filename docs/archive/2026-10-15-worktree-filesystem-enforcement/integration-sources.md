# Existing filesystem-service integration paths — 2026-10-15

Source-only follow-up. No runtime calls, mount probes, builds, installations or code changes. Current Pinfold spec read from `<checkout>/docs/ARCHITECTURE.md`; HEAD observed `198600b1751d889d354ed3657d8f839e8436e469`, with no assertion that the checkout is clean. It still refuses linked worktrees, starts init as the host user with empty capability sets, protects host Git/config, and uses Apple Container on Mac. Existing case-insensitive worktrees and no user volume setup remain requirements.

## Strongest next integration path

**SSHFS in the Linux guest, speaking SFTP over the existing forwarded Unix stream, with an enforcing native host SFTP backend.** This reuses a filesystem client and protocol instead of creating both. It needs neither macFUSE nor a host mount/volume, nor SSH keys, SSH login service, or host TCP listener. It does require new host authorization code and a trusted guest mount setup step. Source inspection establishes feasibility seams, not functioning Pinfold integration or complete security.

Host writer: the custom RequestServer handlers, running with the user's ordinary host identity, would open and modify the actual existing worktree. The parser does not need ambient host filesystem access. `pkg/sftp` FileWriter/OpenFileWriter return caller-owned ReaderAt/WriterAt objects, so data writes can stay bound to an opened `*os.File`. FileCmder owns namespace operations. This is a stronger integration seam than path-only interception: authorization can happen after non-destructive open and before fd-based writes/truncation.

**Important limitation:** RequestServer currently converts FSTAT and FSETSTAT into fresh path-only requests, discarding opened-handle identity for those operations. Supporting correct stat/truncate/chmod semantics after rename/unlink requires a narrow library change or an explicit refusal. Passing an ordinary filesystem example backend through unchanged is not sufficient.

A possible host implementation building block is Go `os.Root`: Unix resolution uses openat-family operations and resists symlink escape. It is not selected-path protection, does not reject hardlink aliases, and does not block crossing filesystem mounts. Go 1.27.2 documents races for Root.Chmod/Chown/Chtimes; fd operations or explicit refusals are needed. A new backend still needs opened-object identity checks, protection of both source/destination namespace operations, safe directory-relative traversal, and a single policy across overlapping exports. Do not treat lexical case folding as equivalent to host name resolution.

## Transport and guest privilege details

Apple Container 1.5.0 (`d265d669ecae041bf338cb3b39c4118316d138f0`) uses Containerization 0.47.0 (`bc994b88df46207fad7775b0eabc51947e315881`). Its SSH socket option is implemented as generic `UnixSocketConfiguration` into the guest. Pinfold already uses that stream for its proxy. Reuse for filesystem traffic therefore needs explicit stream multiplexing or another supported socket mapping; it cannot silently replace egress. No new public network route is necessary.

SSHFS supports **passive** mode directly over stdin/stdout, forcing foreground operation. A trusted launcher can connect the forwarded Unix socket and arrange stdio before exec, avoiding an SSH command subprocess entirely. SSHFS also explicitly accepts `/dev/fd/N` unchanged as a mountpoint. Its source comment calls this pre-mount support for fully unprivileged operation with libfuse >=3.3.0.

Libfuse `fuse_session_mount` accepts an already-mounted `/dev/fuse` descriptor through that syntax and skips mount setup. The existing `mount.fuse3` helper demonstrates opening/mounting first and dropping privileges before filesystem exec. Its implementation clears the capability bounding set, sets/locks securebits and enables no_new_privs. This supports a design with no long-lived uid0 process, no privileged filesystem daemon and no setuid fusermount in the image. It does **not** make the initial mount unprivileged.

Pinfold's current init starts nonroot with `--cap-drop ALL`, so it cannot perform that mount unchanged. A narrowly trusted pre-start stage must create the mount and pass the descriptor before user/capability drop. Containerization's vmexec RunCommand has a privileged mount phase followed by descriptor closing, UID/GID and capability changes; any runtime-level FD handoff needs explicit allowance across that close-on-exec sweep.

**Do not assume an init-side drop solves future exec privileges.** Apple Container 1.5.0 calculates each exec process's capabilities from stored container capAdd/capDrop configuration. Adding SYS_ADMIN to create the mount and dropping it only inside PID1 leaves that stored configuration available to later runtime execs. Either runtime-owned pre-start mounting must keep the workload/exec capability configuration empty, or every relevant entry point needs a separate proven drop before user-controlled code. This remains a concrete integration question, not a demonstrated safe shortcut.

Mount flags, group handling, descriptor closure, lifecycle/readiness, failure behavior and runtime exec semantics need real verification. Guest SSHFS cannot be the host security authority: a compromised guest may send arbitrary SFTP requests. The host backend must enforce all rules independently. Host and guest edits need an explicit cache/coherence policy; SSHFS exposes directory-cache controls and direct_io, with costs unmeasured here.

## Second path: kernel NFS client and native Rust service

`nfsserve` exposes file-ID operations: write and setattr receive IDs; remove/rename receive parent-directory IDs and names. A backend can map those to pinned descriptors and enforce policy at the native host write boundary. It avoids a guest FUSE userspace daemon and fits Pinfold's Rust/Tokio stack more directly.

Costs: its shipped listener is TCP-specific, so avoiding a host listening port requires a Unix-stream transport adaptation. The guest kernel client still needs loopback TCP connected through a relay and a trusted mount step. NFS cache semantics, loss/reconnect behavior, stable file IDs, fsync/durability and complete editor/build behavior need proof. The project explicitly calls its implementation incomplete. It does not include the required enforcing host passthrough filesystem. Treat this as a reserve path, not a mature security solution.

Current Containerization main kernel config enables FUSE and NFSv3, but disables NET_9P. This was read at current-main commit below, **not** verified against the installed kernel image. Therefore NFS availability remains a runtime check; 9p at least carries a likely kernel-build burden.

## Deprioritized alternatives

- **9p:** hugelgupf/p9 has attractive File handles, ReadAt/WriteAt and parent-relative RenameAt/UnlinkAt APIs; server Handle accepts streams. However the inspected Apple kernel configuration excludes 9p, unlike FUSE/NFS. A custom kernel is a larger deployment change before filesystem policy is addressed.
- **macOS virtiofsd:** `mheese/macosvirtiofsd` supplies actual native macOS host file I/O and descriptor-oriented passthrough code. Its transport is QEMU vhost-user with shared VM memory, not Apple's `VZSharedDirectory` API. The README requires custom QEMU with vhost-user enabled (Homebrew QEMU insufficient) and additional Rust-vmm components. No public Apple Container plug-in seam for this backend was established. Adopting it means a substantially different runtime; it still needs the selected-path authorization policy. Default guest UID/xattr mapping also changes host-file semantics unless configured deliberately.
- **Matchlock transport/provider reuse:** it supplies guest FUSE + host CBOR/vsock service, but adopting just its transport still requires guest mounting/startup work and a new provider. The standard SFTP client/server path has a narrower reusable protocol seam and avoids importing the whole Matchlock runtime. This follow-up did not rerun or extend the previous stock-provider probe.
- **Stock OpenSSH sftp-server, host NFS export, or Seatbelt-wrapped service alone:** none of the inspected material establishes opened-identity protection for selected Git/config objects. They are transports or coarse export boundaries, not the missing policy.

## Trust assumptions that a prototype must state

Ordinary concurrent host source edits can remain useful, but metadata topology changes cannot quietly be considered harmless. Replacing the gitfile, moving protected directories, changing effective hooks/config, or adding host aliases can invalidate a policy assembled earlier. A service-side lock serializes its own clients, not arbitrary host Git/editor processes. An initial bounded version may need to refuse/fail closed on authority-layout changes, prohibit guest hardlink creation and unsupported special files, and define what trusted host changes are allowed. It must not claim complete protection against arbitrary concurrent host metadata replacement without a proof. No user volume setup is implied by either leading path.

## Pinned primary sources

- SSHFS `a5d0c9b3b66ae00c1b97c95dbc00789f77b89dd0`: [passive connection and fd mount parsing](https://github.com/libfuse/sshfs/blob/a5d0c9b3b66ae00c1b97c95dbc00789f77b89dd0/sshfs.c#L1268), [fd mountpoint support](https://github.com/libfuse/sshfs/blob/a5d0c9b3b66ae00c1b97c95dbc00789f77b89dd0/sshfs.c#L3967), [options/coherence](https://github.com/libfuse/sshfs/blob/a5d0c9b3b66ae00c1b97c95dbc00789f77b89dd0/sshfs.rst).
- libfuse `423235f673ffe2a0684b270bdbf955e41ab881d0`: [pre-mounted FD support](https://github.com/libfuse/libfuse/blob/423235f673ffe2a0684b270bdbf955e41ab881d0/lib/fuse_lowlevel.c#L5422), [privilege-drop helper](https://github.com/libfuse/libfuse/blob/423235f673ffe2a0684b270bdbf955e41ab881d0/util/mount.fuse.c#L196), [mount handoff](https://github.com/libfuse/libfuse/blob/423235f673ffe2a0684b270bdbf955e41ab881d0/util/mount.fuse.c#L601).
- pkg/sftp `16147c5dd9bba33f5d87a8693d630d169c558932`: [backend interfaces](https://github.com/pkg/sftp/blob/16147c5dd9bba33f5d87a8693d630d169c558932/request-interfaces.go), [handle metadata converted to paths](https://github.com/pkg/sftp/blob/16147c5dd9bba33f5d87a8693d630d169c558932/request-server.go#L271), [dependencies](https://github.com/pkg/sftp/blob/16147c5dd9bba33f5d87a8693d630d169c558932/go.mod). Go, x/crypto, x/sys and kr/fs; no SSH transport needs to be configured for a RequestServer over an owned stream.
- Apple Container 1.5.0 `d265d669ecae041bf338cb3b39c4118316d138f0`: [socket construction](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/Services/RuntimeLinux/Server/RuntimeService.swift#L1181), [exec capabilities](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/Services/RuntimeLinux/Server/RuntimeService.swift#L1291), [dependency pin](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Package.resolved).
- Apple Container current-main inspected `22029b06867e46baf77b3404b849f4491739da3b`; Containerization current-main inspected `fcc92dd69716dee17407acc015e53854d45e9d1b`: [guest privileged setup sequence](https://github.com/apple/containerization/blob/fcc92dd69716dee17407acc015e53854d45e9d1b/vminitd/Sources/vmexec/RunCommand.swift#L252), [generic socket API](https://github.com/apple/containerization/blob/fcc92dd69716dee17407acc015e53854d45e9d1b/Sources/Containerization/UnixSocketConfiguration.swift), [kernel configuration](https://github.com/apple/containerization/blob/fcc92dd69716dee17407acc015e53854d45e9d1b/kernel/config-arm64).
- Go 1.27.2 [Root API](https://pkg.go.dev/os@go1.27.2#Root), [native source](https://github.com/golang/go/blob/go1.27.2/src/os/root.go), [traversal-resistant API rationale](https://go.dev/blog/osroot). Installed source read at `/opt/homebrew/Cellar/go/1.27.2/libexec/src/os/root.go`.
- nfsserve `327b45530f04664da678297ce2568c47bf3fbf81`: [filesystem trait](https://github.com/huggingface/nfsserve/blob/327b45530f04664da678297ce2568c47bf3fbf81/src/vfs.rs), [TCP listener](https://github.com/huggingface/nfsserve/blob/327b45530f04664da678297ce2568c47bf3fbf81/src/tcp.rs), [dependencies](https://github.com/huggingface/nfsserve/blob/327b45530f04664da678297ce2568c47bf3fbf81/Cargo.toml).
- p9 `3fd948847ea46b413f8b98c8f226c6bb716807be`: [File API](https://github.com/hugelgupf/p9/blob/3fd948847ea46b413f8b98c8f226c6bb716807be/p9/file.go), [stream server](https://github.com/hugelgupf/p9/blob/3fd948847ea46b413f8b98c8f226c6bb716807be/p9/server.go#L609).
- macosvirtiofsd `fef121023b40ddebdbd8165509088f6f549ddb6e`: [requirements/transport](https://github.com/mheese/macosvirtiofsd/blob/fef121023b40ddebdbd8165509088f6f549ddb6e/README.md), [native passthrough](https://github.com/mheese/macosvirtiofsd/blob/fef121023b40ddebdbd8165509088f6f549ddb6e/src/fuse/passthrough.rs), [dependencies](https://github.com/mheese/macosvirtiofsd/blob/fef121023b40ddebdbd8165509088f6f549ddb6e/Cargo.toml).

## Cleanup

Only eight owned source clones were created for this follow-up, all under `/private/tmp/pinfold-alias-fs-20261015-`: sshfs, sftp, virtiofs, container, libfuse, nfsserve, containerization and p9. Their exact paths were removed after this note was written, with absence verified. No built binaries, caches, images, boxes, mounts, processes or privileged setup were created. Remaining follow-up artifact: this Markdown note only. Older research resources are outside this follow-up's ownership scope.

## Rust SFTP follow-up: prefer a Rust protocol backend for Pinfold

Source-only check, requested after the preceding comparison. **Prefer investigating `russh-sftp` for production integration rather than adding a Go runtime/helper because the disposable prototype used Go.** It preserves the relevant opaque-handle seam that pkg/sftp's RequestServer loses. This changes the preferred protocol library, not the conclusion that host policy and privileged guest setup remain design work.

The published version is **3.0.1**, released **2026-09-28**, not yanked, **Apache-2.0**. Current repository HEAD was `2ef937985fa965a3e1a8a83828ce7acab30719d6` (2026-10-01). The crates.io 3.0.1 source archive's `.cargo_vcs_info.json` identifies release source **`e8de8f903cb019f32beb26cc1195c7c71bef121f`**. The API points below were checked both in HEAD and that published archive. Recent publication plus the subsequent commit demonstrate current maintenance activity; they are not evidence of a security audit or future support commitment. crates.io reports no declared rust_version/MSRV.

| Property | russh-sftp 3.0.1 | Previously inspected pkg/sftp RequestServer |
|---|---|---|
| File content operations | Handler `read`/`write` receive opaque `handle: String`. Application owns mapping to an opened file. | ReaderAt/WriterAt returned by application can retain an opened file. |
| FSTAT | Separate `fstat(id, handle)` callback, handle preserved by packet dispatcher. | Constructs a new path-only Stat request from the original path. |
| FSETSTAT | Separate `fsetstat(id, handle, attrs)` callback, handle preserved. | Constructs path-only Setstat request, losing opened-file identity. |
| Path metadata | Separate stat/lstat/setstat path callbacks. | Path callbacks. |
| Namespace operations | Handler receives explicit old/new names for rename; explicit symlink, remove, mkdir, rmdir methods. | FileCmder routes path requests. |
| Non-SSH transport | `run`/`run_with_config` accept any AsyncRead + AsyncWrite + Unpin + Send + 'static stream. | NewRequestServer accepts an io.ReadWriteCloser. |

Therefore an existing Tokio UnixStream is the right type of stream for russh-sftp. No SSH daemon, authentication exchange, keys or `russh` dependency is required for this transport. The Handler can keep an application-owned map from session-local tokens to owned file descriptors, including metadata requests after rename/unlink. The library supplies protocol dispatch; it does **not** implement the protective filesystem policy automatically.

Native runtime dependencies are Tokio, tokio-util, serde, serde_bytes, bitflags, thiserror, chrono, bytes, log and dashmap. async-trait is optional. `russh` is a **dev-dependency only**, used for SSH examples; wasm dependencies do not apply to native Pinfold. These additions still need justification against Pinfold's explicit dependency contract. No dependency resolution, build or benchmark was run.

Three limitations matter before adoption:

1. **Lossy filename decoding:** `TryBuf::try_get_string` uses `String::from_utf8_lossy` after invalid UTF-8. Paths and handles use String. An application Handler cannot recover or reject the original invalid bytes after that conversion. Byte-exact support requires protocol adaptation; alternatively reject invalid UTF-8 at the parser before callbacks and declare such layouts unsupported. Do not silently operate on the substituted name. ASCII session-generated handles are straightforward, but hostile handle input must still fail closed.
2. **Lifecycle ownership:** `server::run_with_config` spawns an internal Tokio task and returns unit, exposing no JoinHandle. Shutdown of the owned stream or a small API adjustment is needed to supervise completion/cancellation and guarantee cleanup. Do not equate returning from `run().await` with session completion. The loop breaks on UnexpectedEof and logs other errors; error-driven shutdown behavior needs explicit review.
3. **Extensions and limits:** unsupported methods return the application's unimplemented error. OpenSSH-style fsync and POSIX rename arrive through the generic `extended` callback; useful editor/build durability and rename semantics require deliberately supported extensions or explicit refusals. Default maximum client packet length is 256 KiB; this alone is not a complete application resource budget. No policy or extension implementation was written here.

Primary sources pinned to the published release:

- [Crates.io release metadata](https://crates.io/api/v1/crates/russh-sftp), [3.0.1 source download](https://crates.io/api/v1/crates/russh-sftp/3.0.1/download).
- [Handler API, including fstat and fsetstat](https://github.com/AspectUnk/russh-sftp/blob/e8de8f903cb019f32beb26cc1195c7c71bef121f/src/server/handler.rs#L92).
- [Dispatcher preserves handles](https://github.com/AspectUnk/russh-sftp/blob/e8de8f903cb019f32beb26cc1195c7c71bef121f/src/server/mod.rs#L61).
- [Generic stream entry point and spawned task](https://github.com/AspectUnk/russh-sftp/blob/e8de8f903cb019f32beb26cc1195c7c71bef121f/src/server/mod.rs#L99).
- [Lossy string decoder](https://github.com/AspectUnk/russh-sftp/blob/e8de8f903cb019f32beb26cc1195c7c71bef121f/src/buf.rs#L20).
- [License and dependency manifest](https://github.com/AspectUnk/russh-sftp/blob/e8de8f903cb019f32beb26cc1195c7c71bef121f/Cargo.toml).

Cleanup: the one new owned source directory `/private/tmp/pinfold-alias-fs-20261015-russh-sftp` contained the clone and published source archive extraction. It was removed after this append and absence verified. No builds, tests, policy implementation, processes, mounts, runtime objects or external configuration changes were created. This note remains the only artifact from this follow-up.
