# Filesystem component research — 2026-10-15

Scope: source comparison and one native macOS component probe. No VM or container was started. No production Pinfold files, installed binary, binding contract, host global configuration, or shared runtime changed.

## Decision

Matchlock demonstrates the relevant architectural place for continuous enforcement: its host VFS service handles writes itself. Its shipped providers and hooks do not supply Pinfold's required alias-complete protection. A native probe of that actual provider reproduced modification of a read-only-routed linked-worktree gitfile through alternative names. Reject stock Matchlock as the solution to this requirement; the result does not disprove a differently implemented host filesystem service.

git-lazy-mount solves lazy fresh working copies, not safe live editing of an existing dirty linked worktree. It is not an integration shortcut for the requested workflow. Its separate durable working-tree state is useful design evidence for the isolated-copy alternative, with explicit workflow and platform compromises.

## Baselines and sources

Pinfold instructions read from `<checkout>/AGENTS.md`; relevant current `docs/ARCHITECTURE.md` controls, exports, Pi Git and guarantee sections read directly. It currently refuses worktrees. The admission scan explicitly does not claim atomic or ongoing enforcement. Existing contracts require host Git metadata and protected editor configuration read-only; commits remain host-owned.

Matchlock fresh shallow clone, clean after probe:
`77e02f643e72dbdaf3af295103a42343a330645e`.

- [Host filesystem provider](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/realfs.go)
- [Mount router](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/router.go)
- [Read-only provider](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/readonly.go)
- [Interceptor and handles](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/interceptor.go)
- [Hook API and matching](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/hooks.go)
- [VFS server](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/vfs/server.go)
- [macOS wiring](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/sandbox/sandbox_darwin.go)
- [Linux wiring](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/sandbox/sandbox_linux.go)
- [Overlay preparation](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/pkg/sandbox/overlay_snapshot.go)
- [Dependencies](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/go.mod)
- [Install requirements](https://github.com/jingkaihe/matchlock/blob/77e02f643e72dbdaf3af295103a42343a330645e/docs/install.md)

git-lazy-mount fresh shallow clone, clean:
`ecb969a95fdfe714b69e28d3df07292f238a52fd`.

- [Design contract](https://github.com/mohsen1/git-lazy-mount/blob/ecb969a95fdfe714b69e28d3df07292f238a52fd/docs/design.md)
- [Working-tree model](https://github.com/mohsen1/git-lazy-mount/blob/ecb969a95fdfe714b69e28d3df07292f238a52fd/docs/worktree-model.md)
- [Implementation](https://github.com/mohsen1/git-lazy-mount/blob/ecb969a95fdfe714b69e28d3df07292f238a52fd/crates/worktree/src/lib.rs)
- [FUSE semantics](https://github.com/mohsen1/git-lazy-mount/blob/ecb969a95fdfe714b69e28d3df07292f238a52fd/docs/fuse-semantics.md)
- [Platform requirements](https://github.com/mohsen1/git-lazy-mount/blob/ecb969a95fdfe714b69e28d3df07292f238a52fd/README.md)

## Mechanism comparison

| Mechanism | Actual writer and state | Workflow fit | Decisive limitation |
|---|---|---|---|
| Matchlock live host VFS | Native host Go process serves guest FUSE over vsock. RealFSProvider calls host `os.OpenFile`, `Rename`, `Remove`, etc. | Existing complete working files visible without copying; host/guest edits reach same files. | Router and rules use lexical case-sensitive names, not resolved object identity. Probe changed protected host bytes. |
| Matchlock overlay | Before VM creation, host recursively copies source into per-box snapshot directory, then exposes snapshot using RealFSProvider. | Source files isolated from ordinary snapshot edits; existing source dirty bytes copied. | Full regular-file `io.Copy`, not reflink or lazy copy-up. No controlled change-import/conflict mechanism established. Host-followed symlinks need separate proof. |
| git-lazy-mount | Linux FUSE projection stores modifications in durable overlay, objects in private partial clone, synthetic read-only root gitfile. Stock Git writes actual private admin metadata. | Fresh isolated coding copy with lazy hydration. | Not an existing host dirty-worktree view; no established host-edit reconciliation. Linux only; commits/Git admin are writable by design. |
| New enforcing filesystem service | Service would own every write, safe name resolution and protected identity, while guest receives only service protocol. | Could preserve desired live existing-worktree semantics. | This is substantive new security-critical code and a new transport/guest filesystem integration, not a policy flag. No candidate here proves it. |

Matchlock uses Apple Virtualization.framework on Apple Silicon and Firecracker/KVM on Linux. Adopting the whole runtime changes Pinfold's fixed runtime contract and operational model. Linux setup may require privileged host provisioning. Go dependencies include VZ bindings, CBOR, go-fuse, container image tooling, nftables, gVisor and SQLite. Reusing only the filesystem service avoids some runtime replacement but still needs guest FUSE/protocol integration and a new trusted native service. These are source-level observations; no installation or integration validation was performed.

git-lazy-mount requires Linux FUSE/libfuse3, `/dev/fuse` and Git >=2.36. Its immutable baseline is a committed tree, while durable overlays carry changed files, tombstones, directories and symlinks. Its synthetic gitfile is separate from the host working copy's original pointer. Hardlinks and special files are unsupported. macOS FSKit is explicitly future-platform material. Existing dirty, staged, untracked and ignored files would need deliberate transfer; a partial clone alone does not preserve them. Public benchmark claims are not Pinfold measurements and are not used to infer cost here.

## Native component evidence

Host: macOS 27.0.1 (26A434), arm64, Go 1.27.2, Git 2.56.0. Fixture on `/private/tmp` was observed case-insensitive by `.git` and `.GIT` yielding identical device/inode/bytes. Real disposable main repository and linked worktree, created with system/global Git config disabled. Imported Matchlock's unmodified `pkg/vfs` directly; no stubs and no copied implementation.

Two configurations were exercised: writable parent plus read-only nested gitfile provider; separately an exact host block hook around RealFSProvider. In both, literal protected access failed and host bytes stayed intact. Case, existing hardlink and existing symlink aliases changed the protected host gitfile. The same inode `666261437`, device `16777233`, link count 2 remained across the direct-write cases. Full before/after bytes and identities are in `result.jsonl`. Ordinary root-level file creation succeeded; restored fixture then passed host Git status and log. Fixture removal was verified by failed stat.

This is evidence at Matchlock's native host write boundary, not a complete guest/VM exploit or Pinfold integration test. No host execution marker or Linux runtime probe was performed. We stopped once the essential protection requirement failed. No performance conclusion follows from this tiny fixture.

Existing hooks cannot provide atomic identity authorization using the exposed callback request alone: it has path, flags, mode, UID/GID and data, but no resolved descriptor/device/inode. A callback can perform a separate path stat; RealFSProvider later resolves and opens independently. `O_TRUNC` takes effect during that open, before an after hook. Handle writes retain the initial pathname, while callback metadata excludes file identity. Consequently a new provider needs safe resolution and authorization tied to the same opened object, plus name-operation protection. Hooks by themselves do not prove a discovery-to-use race boundary. This is a source finding, not a new race experiment.

## Reproduction build command

Retain the existing `main.go`, `go.mod`, `go.sum`, and `result.jsonl` from `/private/tmp/pinfold-matchlock-probe-20261015`. The Go module has a local replace to the exact clean Matchlock clone above. Build/run command used:

```sh
cd /private/tmp/pinfold-matchlock-probe-20261015
GOPATH=/private/tmp/pinfold-matchlock-probe-20261015/gopath GOCACHE=/private/tmp/pinfold-matchlock-probe-20261015/cache GOMODCACHE=/private/tmp/pinfold-matchlock-probe-20261015/modcache go run -mod=mod . > result.jsonl
```

The first build attempt omitted the isolated GOPATH and failed reading a sumdb location; the successful command above isolates all Go caches. No upstream patch was necessary. If moving the retained script, adjust only local module replacement/cache paths and check out the pinned source commit.

## Owned resources and cleanup handoff

No running process, VM, container, image, mount, or runtime daemon was created. The probe removed its synthetic main repository and linked worktree and emitted `fixture_removed:true`.

Owned temporary directories still present for the parent to archive/remove:

- `/private/tmp/pinfold-research-matchlock-20261015`: 3.4 MiB, clean source clone.
- `/private/tmp/pinfold-research-glm-20261015`: 1.7 MiB, clean source clone.
- `/private/tmp/pinfold-matchlock-probe-20261015`: 72 MiB. Of this, build cache 70 MiB, module cache 1.4 MiB, isolated GOPATH 4 KiB. Four compact evidence files each occupy one 4 KiB filesystem block.
- This notes file: `/private/tmp/pinfold-worktree-research-20261015-fs.md`.

Memory was used only to identify the established historical boundary and then current spec/source were read. Relevant registry: MEMORY.md:377-388, rollout 01a11c27-0b55-7472-9aeb-0260adb333c9. No current upstream findings depend on memory.
