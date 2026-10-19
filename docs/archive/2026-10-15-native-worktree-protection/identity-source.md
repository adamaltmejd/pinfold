# Apple native sharing identity and policy seams — 2026-10-15

Read-only source/API investigation. No runtime settings, accounts, helpers, ACLs, or mounts were changed by this subtask. Parent task owns any separate runtime experiments.

## Result

Apple's native VirtioFS implementation already enforces the effective host user's filesystem permissions. It does not provide a public per-share host UID/GID selector or operation authorization callback. In stock Apple Container the effective identity is the interactive user's identity. Changing the guest process UID does not change it.

Therefore ACLs can enforce an alias-safe boundary only if they discriminate a genuinely different host writer identity, or if the same restrictions on the human's host operations are acceptable. A deny entry for the current host user affects host Git and editor processes too; it is not a guest-only restriction. This is a consequence of the documented identity model, not a substitute for native guest tests of the exact ACL operations.

A separate native writer identity is a conventional plausible architecture. It could have automated provisioning, so no manual volume/account commands are inherently required. However, selecting a new host identity requires privileged launch/install authority (or an already provisioned account). It is not available as an unprivileged per-container mount flag. An automatically installed helper still requires an administrator authorization step through supported macOS deployment mechanisms. No separate-identity VM was installed or tested here.

## Verified source chain

- Apple Container 1.5.0: `d265d669ecae041bf338cb3b39c4118316d138f0`.
- Its Package.resolved pins Containerization 0.47.0: `bc994b88df46207fad7775b0eabc51947e315881`.
- Local inspected SDK: macOS 27.0 at `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk`.

[Container pinned dependency](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Package.resolved#L14-L20).

Containerization assembles all native shares into a `VZMultipleDirectoryShare`. Each entry is only a host URL and `readOnly: mount.options.contains("ro")`, then one `VZVirtioFileSystemDeviceConfiguration(tag: "virtiofs")`. The `.virtiofs` runtime options array supplies no separate backend policy; its per-mount configure branch does nothing because configuration is centralized.

[Native share construction](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/VZVirtualMachineInstance.swift#L494-L513); [per-mount branch](https://github.com/apple/containerization/blob/bc994b88df46207fad7775b0eabc51947e315881/Sources/Containerization/Mount.swift#L300-L316).

The public `VZSharedDirectory` SDK header exposes `initWithURL:readOnly:`, immutable URL and readOnly properties. `VZVirtioFileSystemDevice` exposes tag and share. There is no identity selector, file-operation callback, alternate virtiofsd socket, or ACL policy configuration in these APIs. That is a bounded statement about the inspected public APIs, not private implementation internals.

[Shared directory API](https://developer.apple.com/documentation/virtualization/vzshareddirectory); [filesystem device configuration](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdeviceconfiguration).

Apple explicitly documents three native filesystem permission rules: file reads/writes use the effective current user's host UID rather than a system process UID; inaccessible host files remain inaccessible; guest requests to change host ownership UID/GID are ignored. Documentation does not say that choosing guest UID 501, root, or nobody changes the host principal. The framework implementation itself is closed source, so the exact mechanism of delegated credential handling was not inspected.

[Apple's permission contract](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdevice). Markdown endpoint inspected with curl: `https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdevice.md`.

## Stock launch identity

Container registers runtime services through `PluginLoader.registerWithLaunchd`, creating a LaunchPlist without UserName or GroupName. ServiceManager selects `gui/<getuid()>`, `user/<getuid()>`, or the system domain according to the session. The generic LaunchPlist type can encode UserName/GroupName, but the runtime registration does not use these fields. It is not evidence of a supported per-container host-user flag.

[Runtime launch registration](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/ContainerPlugin/PluginLoader.swift#L253-L296); [domain selection](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/ContainerPlugin/ServiceManager.swift#L124-L136); [generic plist type](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/ContainerPlugin/LaunchPlist.swift#L29-L54).

The CLI --user/--uid/--gid settings feed ProcessConfiguration, then LinuxProcessConfiguration. They configure guest process credentials, not the launchd runtime identity.

[Guest process parsing](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/Services/ContainerAPIService/Client/Parser.swift#L308-L329); [guest user assignment](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/Services/RuntimeLinux/Server/RuntimeService.swift#L1245-L1262).

The native --mount parser accepts type/source/destination/readonly, plus size/mode only for tmpfs. There is no host user mapping option. Unknown directives fail. The short --volume options array is less restrictive syntactically, but native source construction still recognizes only `ro` for VZSharedDirectory; passing a uid string is not a host identity mechanism.

[Mount parser](https://github.com/apple/container/blob/d265d669ecae041bf338cb3b39c4118316d138f0/Sources/Services/ContainerAPIService/Client/Parser.swift#L387-L465).

Read-only live process observation, `ps -axo pid,ppid,uid,ruid,gid,rgid,comm`, showed:

| PID | Process | UID/RUID | GID/RGID |
|---|---|---|---|
|33351|container-apiserver|501/501|20/20|
|33365|container-network-vmnet|501/501|20/20|
|33368|machine-apiserver|501/501|20/20|
|33392|container-core-images|501/501|20/20|
|87101|container-runtime-linux|501/501|20/20|
|87102|com.apple.Virtualization.VirtualMachine|501/501|20/20|

The last two correspond to an already running VM; this subtask did not start it. This verifies current process identity, not every future launch configuration.

## Why an identity switch is a deployment change

The local macOS `seteuid(2)` manual restricts an unprivileged process to its real/saved identity; it cannot select an unrelated fresh UID. `setgroups(2)` requires super-user privilege. Local `launchd.plist(5)` documents UserName/GroupName as applicable only to the privileged system domain and ignored for agents. Consequently changing the user LaunchAgent plist is not a supported way to obtain a different account.

Apple's supported SMAppService LaunchDaemon registration requires administrator approval before bootstrap. This could be one setup approval in an automatic installer rather than manual filesystem work, but it cannot honestly be described as zero authorization or a change confined to the existing unprivileged CLI. A CLI-only installer could use a privileged package/helper instead; that is still a deployment and trusted-code change.

[SMAppService register](https://developer.apple.com/documentation/servicemanagement/smappservice/register()); [helper migration](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos).

## ACL design issues left to establish

macOS chmod(1) describes ACL entries as user/group allow/deny rules on filesystem objects, with rights including write, append, delete, writeattr, writeextattr, writesecurity, chown, and directory add_file/add_subdirectory/delete_child. Protecting content alone is insufficient: replacement, removal, renaming ancestors, and permission changes must also be prevented. Host atomic replacement of protected files must preserve the policy. Inherited entries apply to new items created below a protected directory, but moved-in objects and newly introduced aliases need explicit testing.

A separate identity should make inode-attached ACL decisions insensitive to case and hardlink spelling, but native sharing must be tested for every mutation and handle lifetime; this source pass does not demonstrate that guarantee. Existing hardlinks to protected files are one reason ACLs may be better than lexical path rules. Symlink targets and parent directory rename rights remain part of the policy.

There are practical consequences to a distinct writer identity: new source files would be owned by that identity, host tools need write access through inherited ACLs/group policy, sensitive ancestor traversal must be granted narrowly, and clean removal must preserve prior ACLs. Shared account use across multiple boxes/projects may collapse isolation between them if any other host interface is available; per-user/per-project identity scope needs a deliberate design.

A separate VM runtime identity might also affect image/cache ownership, runtime Mach services and client authorization, forwarded proxy/socket access, and privacy-protected folders. These are integration questions, not proven blockers. There is no evidence here for selecting only the private VZ filesystem worker's identity independently of the owning runtime.

No source-only estimate proves native ACL performance. It would avoid the SSHFS round-trip implementation, but normal native share performance must be remeasured under the actual ACL/runtime design.

## Scope and cleanup

This subtask read the binding threat model in `<checkout>/docs/ARCHITECTURE.md`, current Apple docs, pinned public source, local SDK/man pages and live process identities. It made no Pinfold changes, runtime changes, account/helper installation, ACL changes, builds, images, or mounts. The two disposable source clones were removed after writing this note; only this compact note remains.
