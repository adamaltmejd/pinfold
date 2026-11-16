# Session end probe, #322, 2026-11-16

Probed committed `fa940fac150f4368092df1459fb803c1dec367bd`. Read BRIEF.md, its RULES.md, AGENTS.md, ARCHITECTURE.md,
PLAN.md and `gh issue view 322`. No implementation, specification, workflow or suite changed. No credential used.
Evidence is in `2026-11-16-session-end-probe/`; O below means observations.txt line numbers. Store measurements are in
store.txt. Output is selected; normal /proc status fields are joined onto one line.

Commands: `RUSTC_WRAPPER= cargo build --locked` on Mac; on Linux add `--target x86_64-unknown-linux-musl`. Both
succeeded. Mac fmt and clippy passed (O101–103). The active trial clone was dirty at `7699232`. Fetched this public HEAD
into a separate `/tmp/pfp322` worktree, preserving the clone; removed that worktree and the manual VMM afterward
(O104–105). The lease remains active, as requested. All Linux access used `labvm exec -- bash -lc '…'`; long jobs were
detached.

## Departures: observed on real kept project boxes

Command: `python3 "$D/sessions.py" "$B" "$IMAGE"`, where D is this evidence directory. Mac IMAGE was
`localhost/pinfold/profile-default:latest`; Linux used its existing
`localhost/pinfold/image-probe-owner-rereview:latest` fixture. The script sets separate config/state roots, stamps
maintenance before dispatch, uses the real cache and `PINFOLD_NO_UPDATE_CHECK=1`, and requests 2 CPUs/1G. It starts with
`pinfold start`. Each `pinfold -- perl workload.pl CASE` launches TERM/HUP/INT-ignoring workers, a double fork, setsid
and continuous bounded forks. It records the leader, four workers and churner's PIDs; each worker requests an allowed
HTTP host fixture. A second session waits for input, then requests it.

O13–20 (Mac) and O36–43 (Linux) list every surviving process and its state for non-TTY and TTY SIGINT, SIGTERM, SIGKILL
and terminal closure. All six named processes survived every case; all four workers made fresh fixture requests after
the client had exited. The second session returned OK in the same generation. Client statuses were 130, 143, -9 and 129.
Closure closes a real controlling PTY; non-TTY keeps terminal stdin but redirects stdout. This is not a simulated HUP.
The final runs sample fixture counters after confirmed client exit. Earlier runs sampled too soon and were replaced.
Each run stopped its box and removed disks.

## Guest and spawner: observations, then source

O2–11: Apple CLI 1.7.1, kernel 6.18.15; cgroup2 mounted rw at /sys/fs/cgroup, root-owned directory 0755 and procs 0644,
cgroup.kill present. Controllers: cpuset cpu io memory hugetlb pids. PID 1 is pinfold, uid:gid 501:20, every cap set
zero, NNP=1. The exec'd shell has the same identity/caps/NNP, PPid=0. O24–34: Linux kernel 6.18.57; same
mount/controllers, root directory 0555, procs root-owned 0644; work belongs to /work, pids.max=2048. PID 1 is root
pinfold, bounding/effective/permitted 0x200000 (SYS_ADMIN), NNP=0. Its child exec service PID 588 and shell PID 622 are
1000:1000, all cap sets zero, NNP=1. Both guests report /proc/sys/user/max_user_namespaces=0 (O4, O26).

Code: init/firecracker.rs:101–137,804–869 forks and drops the service; 930–1054 spawns exec children; 1101–1140 closes
input on disconnect without signalling. Lines 370,454–459 establish cgroup2 and /work; 779–791 uses /work/cgroup.kill
only for whole-box unmount. These match the observed parent. Code: core/runtime/apple.rs:84–140 fixes host uid, cap-drop
ALL and pinfold init; 874–911 uses independent container exec. An image entrypoint cannot override it.

For Apple's external parent, ran `python3 "$D/apple.py"`, then `container run -d --name probe-s322-parent --network none
--cap-drop ALL --user 501:20 --kernel-arg sysctl.user.max_user_namespaces=0 --init-image
localhost/probe-s322-vminit:latest --entrypoint /bin/sleep localhost/pinfold/profile-default:latest 600`, and `container
exec probe-s322-parent /bin/true`. The custom init wraps the existing 0.53.0 vmexec, prints /proc status and execs the
unchanged helper. O98–99 observes vmexec's parent vminitd, VM PID 1: both root, bounding/effective/permitted
0x1ffffffffff, NNP=0. This instrumented VM observation is distinct from the ordinary container's PPid=0 observation. An
attempted Containerfile COPY reported a diffID mismatch (O115); apple.py changes diffID metadata only in its separate
probe image.

Documentation: installed `container help run` exposes --entrypoint, --cap-add, --cap-drop and --init-image; exec exposes
--user/--uid/--gid but no capability, cgroup or tree-kill option. [Apple 1.7.1
reference](https://raw.githubusercontent.com/apple/container/1.7.1/docs/command-reference.md), run lines 47–59, exec
325–351, kill 274–291, documents those scopes. [ManagedProcess
0.53.0](https://raw.githubusercontent.com/apple/containerization/0.53.0/vminitd/Sources/VminitdCore/ManagedProcess.swift)
183–185 places exec into the owning process's cgroup; 286–300 signals the stored PID (web-rendered line numbers).
[ExecCommand](https://raw.githubusercontent.com/apple/containerization/0.53.0/vminitd/Sources/vmexec/ExecCommand.swift)
68–182 enters namespaces, forks, setsid and drops credentials. Pinning source 0.54.0 from the CLI package gave the same
relevant code; the instrumented init used 0.53.0.

## Controller, alternatives and cost: observed

Mac: set ZIG_GLOBAL_CACHE_DIR and ZIG_LOCAL_CACHE_DIR under /tmp/pfp-zig for reproduction. Compile controller.c with
`zig cc -target aarch64-linux-musl -static -O2 -DWORK_UID=501 -DWORK_GID=20 -DKEEP_KILL -DALTERNATIVES "$D/controller.c"
-o /tmp/pfp-controller-arm-alt4`. Run it mounted read-only at /probe with `container run --rm --network none --read-only
--kernel-arg sysctl.user.max_user_namespaces=0 --cap-drop ALL --cap-add SETPCAP --cap-add SETUID --cap-add SETGID
--cap-add KILL --cap-add SYS_ADMIN --tmpfs /tmp --volume /tmp/pfp-controller-arm-alt4:/probe:ro --entrypoint /probe
--name probe-s322-final localhost/pinfold/profile-default:latest`. SYS_ADMIN is for the PID-namespace comparison.
Earlier cgroup-only runs omitted it. Linux: copy controller.c/manual-linux.sh into /tmp/pfp322-probe, then
`PROBE_CFLAGS='-DKEEP_KILL -DALTERNATIVES' bash /tmp/pfp322-probe/manual-linux.sh`. This is a separate networkless root
Firecracker VM using the same pinned kernel, not a production-init integration or a changed pinfold binary.

O47–91: the controller creates a parent before dropping the bounding set to zero. The normal Linux guest's 0555 root
(O31) does not establish the manual VM's permissions. Its exact setup privilege requirements are unestablished. Steady
controller E/P=0xe0: SETUID, SETGID, KILL; NNP=1. That tested envelope created groups, placed a child before exec and
wrote cgroup.kill. SETGID clears supplementary groups and sets gid; SETUID sets uid. SETPCAP was transient for clearing
the bounding set. After exec work has the actual box uid:gid, every cap set zero and NNP=1. Writes to root/own procs and
creating an outside group fail EACCES. No writable membership file is delegated to work. With only SETUID/SETGID (0xc0),
all six TERM attempts fail EPERM (O93–96). With KILL, no EPERM was recorded; other TERM errors were not counted, so
historical TERM delivery is unestablished. A monotonic timer waits two seconds, then cgroup.kill ends every
fork/setsid/double-fork descendant and events reports populated 0. A separately contained peer stays alive. Return 0
means success; printed errno on successful calls is stale. This is the least tested envelope, not a minimum proof.

Mac final empty mkdir+rmdir: mean 4 us over 1000; kill write 30 us, empty at 89 us. Linux: 22 us, 152 us, empty at 1647
us (O63–67, O86–90). Cleanup also includes the deliberate two-second grace and a final rmdir. Both created 256
concurrent empty scopes; max.descendants reports max. This is a lower bound, not 256 populated executions or a measured
maximum. Linux's production /work shares its 2048-task limit; boot, IPC and full dispatch cost were not benchmarked. No
finite cgroup-count ceiling was established.

PID namespace: with SYS_ADMIN, killing its init ends all 6–7 visible members; none remain (O48, O71). Without privilege
creation fails EPERM (O56, O79). Subreaper: recursive /proc status traversal snapshots one child; after the scan that
child double-forks. Killing the snapshot misses the new child, observed alive and adopted by the subreaper (O49, O72).
This demonstrates that race, not that every repeated scanner must fail. The retained portable status scan passed on
both. Apple's documented kill and inspected implementation offer no complete exec scope; the live alternative was
killing PID-namespace init, which ends its scope.

## Store, cleanup and scope

store.txt lists every exposed image name with compressed bytes, content allocation and unpacked allocation, grouping
aliases. Every pinfold name has no container reference; that does not establish absence of project/profile users. No
volumes. Final runtime totals: images 15,698,374,656 bytes; buildkit 3,020,423,168 bytes. Actual guest
/var/lib/buildkit: 2,310,992 KiB; host builder exports: 483,720 KiB. APFS sharing prevents adding per-image allocations
as exclusive reclaimable space. The retained snapshot shows shared buildkit running; automatic startup and cache-growth
attribution are unestablished. No shared image, container, volume or cache was pruned. The named predecessor image was
already absent on Mac (O106). Removed the probe init image and all probe boxes/disks; retained no new image. The reused
Linux fixture image and dirty clone remain. Historical retained Mac disk readings are 60 GiB and 56 GiB (store.txt); no
continuous disk minimum is established. The suspended suite was not run. No owner lease/controller integration,
concurrent populated-scope stress, TERM pidfd delivery, hostile root workload or arbitrary image compatibility is
proved.

## Review follow-up: only independent exec privilege and populated cost rerun

The existing Apple service and trial lease were available. Before compilation, Mac disk was 60 GiB and Linux 25G
(O117–119). No other probe cases were rerun. The historical empty-cgroup timings do not answer populated execution cost.

Compile controller.c as before with `-DPOPULATED_COST`, omitting `-DKEEP_KILL -DALTERNATIVES`, into
`/tmp/pfp-review/cost-arm`. Run `container run --rm --name probe-s322-review-cost --network none --read-only
--kernel-arg sysctl.user.max_user_namespaces=0 --cap-drop ALL --cap-add SETPCAP --cap-add SETUID --cap-add SETGID
--cap-add KILL --user 0:0 --volume /tmp/pfp-review/cost-arm:/probe:ro --entrypoint /probe
localhost/pinfold/profile-default:latest`. Linux uses manual-linux.sh with `PROBE_CFLAGS=-DPOPULATED_COST`.

After COST-DONE, run `container exec probe-s322-review-cost /bin/sh -c 'for p in 1 $$; do echo PROCESS=$p; cat
/proc/$p/status | sed -n "/^Name:/p;/^Uid:/p;/^Gid:/p;/^CapPrm:/p;/^CapEff:/p;/^CapBnd:/p;/^NoNewPrivs:/p"; done'`. Read
the exec shell again using `container exec --user 501:20` and `/proc/$$/status` with the same field selection. O121–145:
the entry controller has B=0, NNP=1; independent root shell E/P/B=0x1e0, NNP=0. With uid:gid 501:20, E/P=0 but B=0x1e0
and NNP=0. This observes that specific run configuration; it does not test every capability or image.

Cost uses CLOCK_MONOTONIC. Creation starts before mkdir and includes fork, membership assignment, identity drop, exec
and all five readiness bytes; populated=1 is checked next. Destruction starts before TERM enumeration and includes the
two-second timer, cgroup.kill, populated=0, available child reaping and successful rmdir. Each sample counts every TERM
call's return/error; unlike the historical loop, these samples record successful calls, not just absence of EPERM.
O147–155: three Mac samples create in 557–15543 us, destroy in 2000167–2000446 us; Linux creates in 1637–8489 us,
destroys in 2000290–2000376 us. Kill-to-removal is 105–131 us Mac, 231–282 us Linux. Mac used 4 CPUs/1G (O151), Linux 2
CPUs/1G. These are sample ranges, not performance bounds or a dispatch benchmark. Boot/setup is excluded. The original
concurrency ceiling remains unestablished. Follow-up boxes and the manual VMM/raw disk were removed; no image was built,
no worktree created, and the lease was retained (O157–159).

## What a design must respect

- Both kernels provide protected cgroup2 membership and descendant-complete kill.
- Current unprivileged spawners cannot create this containment or deliver privileged TERM.
- Linux PID 1's current privilege/role contract excludes the demonstrated controller.
- Apple exec is runtime-spawned; a root entry command and a custom VM init are available.
- The tested elevated Apple run configuration gives an independent root exec E/P/B=0x1e0, NNP=0 (O129–136).
- Cgroup ownership and pre-exec placement were used in the successful probes.
- SIGKILL and terminal closure leave guest work and fixture access alive today.
- Tree snapshots race; privileged PID namespace teardown ended every observed member.
