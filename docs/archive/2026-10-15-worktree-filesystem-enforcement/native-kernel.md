# Native filesystem broker confinement, 2026-10-15

## Conclusion

Seatbelt on the **actual native host filesystem writer** is useful, but not complete inode protection. A stricter allowlist written with `require-all` and `require-not`, plus denial of new hardlink creation, still permits writes through existing hardlinks and already-open writable file descriptors. It also permits writes after the trusted host moves an ordinary open file to the protected name. The case-insensitive filename checks themselves worked in these native probes, including absent names.

A VM can keep its Linux tools while a separately sandboxed native broker handles every host write. This differs materially from applying Seatbelt to the Virtualization.framework VM owner. However, that broker needs its own descriptor/identity and namespace checks; Seatbelt is defense in depth, not a replacement. No universal unprivileged macOS per-inode deny policy was established. Nothing in this report proves a VM integration secure.

## Native experiment and observations

Platform: arm64 macOS 27.0.1, build 26A434. The actual writing process called `sandbox_init` itself. Parent processes outside that new sandbox inspected bytes, mode, inode number and link count after every operation. The parent used pipes, not sleeps, to introduce deterministic post-open host changes. All files were disposable fixtures inside `/private/tmp/pinfold-alias-kernel-20261015`.

Codex's existing sandbox prevents nested `sandbox_apply`; the initial probe returned EPERM. The same explicitly authorized disposable commands then ran outside that surrounding sandbox with automatic approval. No privilege elevation to root, settings changes, runtime services, VM or container actions occurred.

Two policies were compared. Policy 0 reproduced the literal-deny control. Policy 1 tested a materially stronger policy shape:

```scheme
(version 1)
(allow default)
(deny file-write*)
(allow file-write*
  (require-all
    (subpath "/private/tmp/.../fixture")
    (require-not (literal "/private/tmp/.../fixture/.git"))))
(deny file-write-unlink file-write-create
  (literal "/private/tmp/.../fixture/.git"))
(deny file-link)
```

The path shape is intentionally minimal to isolate file semantics. It is not a production broker profile: network, process creation and other operations were left allowed. Both policies produced the same results below.

| Operation in actual native writer | Result | Host observation |
|---|---|---|
| Ordinary root file write | Allowed | Ordinary bytes became EDIT; protected file stayed SAFE |
| Open/write `.GIT` alias of `.git` | EPERM | Protected bytes and inode unchanged |
| Write existing ordinary-name hardlink to `.git` | Allowed | Same protected inode, link count 2, bytes became EDIT |
| Create new hardlink from `.git` | EPERM | No new alias; protected bytes unchanged |
| Unlink `.GIT` | EPERM | Protected name, inode and bytes unchanged |
| Rename `.GIT` away | EPERM | Protected name, inode and bytes unchanged |
| Rename replacement over `.GIT` | EPERM | Original inode and bytes unchanged |
| Write symlink pointing to `.git` | EPERM | Protected bytes unchanged |
| `pwrite` protected FD opened before sandbox application | Allowed | Same protected inode became EDIT |
| Open ordinary FD after sandbox; parent renames it over `.git`; `pwrite` | Allowed | New protected inode became EDIT |
| Parent adds hardlink to `.git` after sandbox readiness; child opens alias | Allowed | Protected inode link count became 2 and bytes became EDIT |
| `chmod` existing hardlink alias | Allowed | Protected inode mode changed 0600 to 0644 |

Representative strengthened-policy evidence:

```text
BEGIN p1 prelink
before inode=667539827 nlink=2 mode=600 bytes=SAFE
RESULT r=4 errno=0
after  inode=667539827 nlink=2 mode=600 bytes=EDIT
BEGIN p1 preopened
before inode=667539854 nlink=1 mode=600 bytes=SAFE
RESULT r=4 errno=0
after  inode=667539854 nlink=1 mode=600 bytes=EDIT
BEGIN p1 host_move_fd
before .git inode=667539858 bytes=SAFE
RESULT r=4 errno=0
after  .git inode=667539859 bytes=EDIT
```

The host-move and host-created-link cases are deliberately adversarial trusted-host changes. Ordinary editing does not necessarily perform them. They establish which assumptions the broker must state, rather than proving that all concurrent host editing is unsafe.

## Case spelling and absent paths

A second native probe used literal `.git` and subpath `.vscode` denials:

- Creating absent `.GIT` was denied.
- Writing an existing file whose actual disk spelling was `.GIT`, with lowercase `.git` in the policy, was denied.
- Creating absent `.VSCODE` was denied.
- The same process successfully wrote the ordinary control each time.

This is current empirical behavior on this filesystem. It does not justify implementing independent lowercase-string path checks in a broker. Filesystem normalization includes more than ASCII case folding. Use trusted root descriptors and host filesystem object identity, and treat the kernel's native path enforcement as an additional layer.

## Immutable inode flags: bounded secondary experiment

A distinct experiment set `UF_IMMUTABLE` (`uchg`) on a disposable protected inode, then sandboxed its writer with `(deny file-write-flags)`. A pre-existing hardlink referred to the same inode. The trusted parent cleared the flag afterward.

| Operation | Result |
|---|---|
| Open/truncate through existing hardlink | EPERM; SAFE preserved |
| Clear flags via alias pathname | EPERM; flags remained 2 |
| Clear flags via existing FD | EPERM; flags remained 2 |
| Unlink alias | EPERM; link count remained 2 |
| Replace protected name | EPERM; inode unchanged |
| `pwrite` via FD opened before setting immutable | **Allowed; EDIT reached protected inode while flags remained 2** |
| Ordinary write control in each process | Allowed |

This is real inode-based protection for new opens and tested namespace mutations, but not revocation of existing writable descriptors. It mutates host repository metadata, blocks legitimate host changes to flagged files, requires ordering before any writable broker handle exists, and needs crash recovery. Flagging all shared Git metadata would interfere with host commits. It is unsuitable as a transparent default and was not applied to any user repository.

## Primary source implications

- Current [Chromium Seatbelt documentation](https://chromium.googlesource.com/chromium/src/+/main/sandbox/mac/README.md) describes sandbox(7) as confinement of the process and its children. It is a different object from a VM's externally serviced shared-directory grant.
- [Codex Seatbelt code at e89b7c442c0433ad5167e72a5a25bb6d0712766a](https://github.com/openai/codex/blob/e89b7c442c0433ad5167e72a5a25bb6d0712766a/codex-rs/sandboxing/src/seatbelt.rs) constructs `require-all` root grants with literal/subpath exclusions, normalizes logical/resolved paths and denies unlink of writable root directory anchors. `vnode-type DIRECTORY` selects a file type; it is not a particular inode's security label. This syntax does not imply hardlink identity protection.
- [Anthropic sandbox-runtime at 9a277e5e04af1ab1b04feea4400a6ed8a5b72039](https://github.com/anthropics/sandbox-runtime/blob/9a277e5e04af1ab1b04feea4400a6ed8a5b72039/src/sandbox/macos-sandbox-utils.ts) emits path filters and ancestor unlink/create restrictions. These stop namespace relocation of protected paths by the sandboxed process. They do not revoke a descriptor or add an unknown hardlink spelling to a pathname deny.
- Local Apple SDK `sys/stat.h` defines `UF_IMMUTABLE` as 0x2; the probe above tests the actual limitation rather than relying on its “file may not be changed” comment.
- Local Apple SDK `EndpointSecurity/ESTypes.h` and `ESClient.h` require root, `com.apple.developer.endpoint-security.client`, and user Full Disk Access approval for an Endpoint Security client. The corresponding [Apple entitlement error](https://developer.apple.com/documentation/endpointsecurity/es_new_client_result_err_not_entitled), [root privilege error](https://developer.apple.com/documentation/endpointsecurity/es_new_client_result_err_not_privileged), and [TCC error](https://developer.apple.com/documentation/endpointsecurity/es_new_client_result_err_not_permitted) document those gates. Endpoint Security is not a privileged-free portable Pinfold primitive and was not installed or tested.
- Apple's supported [App Sandbox file-access API](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox) grants user-selected scoped access. It does not document an API to attach a custom per-inode deny label to arbitrary existing worktree files. Absence of such a documented API is a research limitation, not proof no private kernel facility exists.

## Practical broker boundary

A separately launched native SFTP/9p/custom broker can retain the Linux VM tool environment if guest filesystem requests go exclusively through that broker. No writable Virtualization.framework host share may bypass it. The native broker must enter its sandbox before opening writable host files and must not inherit writable descriptors. A guest-root mount setup or FUSE capability decision remains separate from this native result.

Conservative broker design constraints worth testing:

1. Open regular files without truncation and without following symlinks, validate the opened descriptor's identity and link count, then mutate that exact descriptor. Never authorize a string and reopen it later. `O_TRUNC` before `fstat` is already a write.
2. Refuse multiply-linked regular files for all data/metadata mutations if complete protected-identity tracking is unavailable. Refuse guest hardlink creation. This is a clear unsupported-layout policy, not a promise to preserve arbitrary hardlink semantics.
3. Namespace operations need anchored parent descriptors and policy checks on source and destination objects/names. Protect the worktree gitfile, effective hook location and protected config roots, plus ancestors whose relocation could move them out of policy. Names and identities solve different problems.
4. Serialize broker namespace mutations or prove cross-request races safe. Authorize every writable handle at creation and define what later host changes mean. A `F_GETPATH` lookup or `fstat` followed by write is not an atomic guarantee against an external host rename/link.
5. State a trusted-host rule: no moving/linking writable guest objects into protected metadata, and no changing Git layout/configuration or hook destinations while a session uses its policy. Normal source edits and atomic saves can remain allowed, but stale-descriptor writes may target an unlinked old inode; define/report that behavior instead of claiming conflict-safe synchronization.
6. If those host assumptions are unacceptable, use a controlled-application boundary or a much stronger filesystem service protocol. Native Seatbelt alone does not supply the missing atomicity.

These are design constraints, not a security proof for any implementation. The experiments used plain files, not actual repositories or host Git execution. They did not test mmap, every metadata syscall, Unicode normalization, full SFTP protocol handling, transport, denial-of-service or a real guest mount. They specifically decide the native path-policy shortcut and identify the remaining enforcement work.

## Reproduction and cleanup

Retained small reproduction sources and raw outputs are under `/private/tmp/pinfold-alias-kernel-20261015`: `probe.c`, `flags.c`, `names.sh`, `results.txt`, `flags-results.txt`, `names-results.txt`. Compile the C sources with the system clang and run outside a pre-existing Seatbelt sandbox. They use the fixed disposable directory shown above; fixture directories must not pre-exist. `flags.c` clears every immutable flag it sets during normal execution; if interrupted, inspect and clear only its six owned `.git` fixtures before removal.

All 33 fixture subdirectories, both compiled executables and downloaded source files were removed after verification. The six immutable fixture files had flags 0 before deletion. All child processes were synchronously reaped. No VM, container, image, mount, user repo or host setting was created or changed. The retained source/log files are research artifacts for archival, not leftover execution resources.
