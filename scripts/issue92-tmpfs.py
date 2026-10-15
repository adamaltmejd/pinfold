#!/usr/bin/env python3
"""Disposable issue #92 store-backing diagnostic; never changes product source."""

import hashlib
import json
import os
import platform
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path


BASE = "36adf1495439f8e3f2714a546af51cdb1e846056"
VENDOR_SHA = "322dffd5a543bb9e1ca9c67cc56999c62b746118674d004180fe60dddfc21979"
ALLOWED = {".github/workflows/ci.yml", "scripts/issue92-tmpfs.py"}
OUT = Path(os.environ["RUNNER_TEMP"]) / "issue92-tmpfs"
GRAPH = Path(os.environ["HOME"]) / ".local/share/containers/storage"
STATE = OUT / "mount-owner.json"


def save(name, value):
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def run(*args, timeout=30):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=timeout).stdout


def mount_info(exact):
    args = ("findmnt", "--json", "--mountpoint" if exact else "--target", str(GRAPH),
            "-o", "TARGET,FSTYPE,SOURCE")
    result = subprocess.run(args, capture_output=True, text=True, timeout=10)
    if result.returncode == 1:
        assert not result.stderr.strip(), "findmnt failed instead of finding no mount"
        return None
    assert result.returncode == 0, f"findmnt failed with status {result.returncode}"
    rows = json.loads(result.stdout)["filesystems"]
    assert len(rows) == 1, "findmnt returned ambiguous mount inventory"
    return rows[0]


def common(mode):
    assert mode in ("control", "tmpfs")
    assert os.environ.get("GITHUB_ACTIONS") == "true" and os.getuid() != 0
    assert platform.machine() == "x86_64"
    assert Path(os.environ["HOME"]).is_absolute() and OUT.is_absolute()
    assert not os.environ.get("XDG_DATA_HOME"), "default graphroot path must be stable"
    assert not os.environ.get("CONTAINERS_STORAGE_CONF"), "storage config override present"
    assert not (Path(os.environ["HOME"]) / ".config/containers/storage.conf").exists()
    assert not GRAPH.is_symlink() and GRAPH.resolve() == GRAPH
    assert os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}") == f"/run/user/{os.getuid()}"


def source(mode, output="source.json"):
    common(mode)
    assert len(os.sched_getaffinity(0)) == 4, "ordinary four-CPU affinity required"
    assert not any(os.environ.get(k) for k in
                   ("RUST_TEST_THREADS", "CARGO_BUILD_JOBS", "CARGO_TARGET_DIR"))
    changed = set(run("git", "diff", "--name-only", "-z", BASE, "HEAD").split("\0")) - {""}
    assert changed <= ALLOWED, f"source differs from fixed base: {sorted(changed - ALLOWED)}"
    assert not run("git", "status", "--porcelain", "--untracked-files=all")
    save(output, {"base": BASE, "head": run("git", "rev-parse", "HEAD").strip(),
                         "changed_paths": sorted(changed), "affinity": sorted(os.sched_getaffinity(0)),
                         "mode": mode, "scope": "diagnostic only; no acceptance claim"})


def prepare(mode):
    common(mode)
    assert not mount_info(True), "graphroot is already a mountpoint"
    GRAPH.parent.mkdir(parents=True, exist_ok=True)
    created = not GRAPH.exists()
    if GRAPH.exists():
        assert GRAPH.is_dir() and not any(GRAPH.iterdir()), "pre-existing graphroot is nonempty"
        prior = GRAPH.stat()
        assert prior.st_uid == os.getuid() and prior.st_gid == os.getgid()
        assert stat.S_IMODE(prior.st_mode) == 0o700, "pre-existing graphroot permissions differ"
    else:
        GRAPH.mkdir(mode=0o700)
    marker = {"mode": mode, "graphroot": str(GRAPH), "uid": os.getuid(),
              "created_graphroot": created,
              "source": f"issue92-tmpfs-{os.environ['GITHUB_RUN_ID']}-{os.environ['GITHUB_RUN_ATTEMPT']}-{os.environ['ISSUE92_SAMPLE']}"}
    save("mount-owner.json", marker)  # written before mount so failed setup remains cleanable
    if mode == "tmpfs":
        opts = f"size=6G,mode=0700,uid={os.getuid()},gid={os.getgid()}"
        run("sudo", "mount", "-t", "tmpfs", "-o", opts, marker["source"], str(GRAPH))
        actual = mount_info(True)
        assert actual and actual["target"] == str(GRAPH)
        assert actual["fstype"] == "tmpfs" and actual["source"] == marker["source"]
    save("prepare.json", {"mode": mode, "mount": mount_info(True),
                          "graphroot": str(GRAPH), "status": "complete"})


def verify(mode):
    common(mode)
    vendor = Path("/usr/share/containers/storage.conf").read_bytes()
    assert hashlib.sha256(vendor).hexdigest() == VENDOR_SHA, "vendor storage config differs"
    info = json.loads(run("podman", "info", "--format", "json", timeout=90))
    save("runtime.json", info)
    store = info["store"]
    assert info["host"]["security"]["rootless"] is True
    assert store["graphRoot"] == str(GRAPH)
    assert store["runRoot"] == f"/run/user/{os.getuid()}/containers"
    assert store["graphDriverName"] == "overlay" and not store["transientStore"]
    assert store["graphOptions"].get("overlay.mountopt") == "nodev"
    assert not store["graphOptions"].get("overlay.mount_program"), "FUSE fallback refused"
    assert store["graphStatus"]["Native Overlay Diff"] == "true"
    mounted = mount_info(True)
    if mode == "tmpfs":
        owner = json.loads(STATE.read_text())
        assert mounted and mounted["source"] == owner["source"] and mounted["fstype"] == "tmpfs"
        assert store["graphStatus"]["Backing Filesystem"] == "tmpfs"
        vfs = os.statvfs(GRAPH)
        assert vfs.f_blocks * vfs.f_frsize == 6 * 1024**3, "tmpfs capacity differs from 6 GiB"
    else:
        assert mounted is None and store["graphStatus"]["Backing Filesystem"] == "extfs"
    save("verification.json", {"mode": mode, "mount": mounted, "status": "complete",
                               "vendor_sha256": VENDOR_SHA})


sampling = True


def stop_sample(*_):
    global sampling
    sampling = False


def cgroup_dir():
    entries = [line[3:] for line in Path("/proc/self/cgroup").read_text().splitlines()
               if line.startswith("0::")]
    if len(entries) != 1:
        return None
    member = Path(entries[0])
    for line in Path("/proc/self/mountinfo").read_text().splitlines():
        before, separator, after = line.partition(" - ")
        fields = before.split()
        if not separator or len(fields) < 5 or after.split()[0] != "cgroup2":
            continue
        root, mount = Path(fields[3]), Path(fields[4])
        try:
            relative = member.relative_to(root)
            candidate = mount / relative
            candidate.relative_to(mount)
        except ValueError:
            continue
        return candidate if candidate.is_dir() else None
    return None


def memory_cgroup(directory):
    if directory is None:
        return {"status": "unavailable"}
    try:
        values = {}
        for key in ("current", "peak", "max"):
            raw = (directory / f"memory.{key}").read_text().strip()
            values[key] = None if raw == "max" else int(raw)
        values["events"] = {name: int(count) for name, count in
                            (line.split() for line in (directory / "memory.events").read_text().splitlines())}
        return {"status": "visible", **values}
    except (OSError, ValueError):
        return {"status": "unavailable"}


def sample(mode):
    signal.signal(signal.SIGTERM, stop_sample)
    signal.signal(signal.SIGINT, stop_sample)
    minimum_memory = None
    maximum_used = 0
    maximum_cgroup_current = None
    maximum_cgroup_peak = None
    count = 0
    cg = cgroup_dir()
    first_oom_kill = None
    last_oom_kill = None
    while sampling:
        vfs = os.statvfs(GRAPH)
        used = (vfs.f_blocks - vfs.f_bfree) * vfs.f_frsize
        mem = next((int(line.split()[1]) * 1024 for line in
                    Path("/proc/meminfo").read_text().splitlines()
                    if line.startswith("MemAvailable:")), None)
        maximum_used = max(maximum_used, used)
        minimum_memory = mem if minimum_memory is None else min(minimum_memory, mem)
        cgroup = memory_cgroup(cg)
        if cgroup["status"] == "visible":
            maximum_cgroup_current = max(maximum_cgroup_current or 0, cgroup["current"])
            maximum_cgroup_peak = max(maximum_cgroup_peak or 0, cgroup["peak"])
            last_oom_kill = cgroup["events"].get("oom_kill")
            if first_oom_kill is None:
                first_oom_kill = last_oom_kill
        print(json.dumps({"kind": "sample", "wall_ns": time.time_ns(), "mode": mode,
                          "filesystem_used_bytes": used, "host_mem_available_bytes": mem,
                          "cgroup_memory": cgroup,
                          "cgroup_scope": "sampler process only; Podman/systemd scopes may differ"}), flush=True)
        count += 1
        time.sleep(1)
    print(json.dumps({"kind": "complete", "samples": count,
                      "max_filesystem_used_bytes": maximum_used,
                      "min_host_mem_available_bytes": minimum_memory,
                      "max_sampler_cgroup_current_bytes": maximum_cgroup_current,
                      "max_sampler_cgroup_peak_bytes": maximum_cgroup_peak,
                      "observed_oom_kill_delta": (last_oom_kill - first_oom_kill
                                                  if first_oom_kill is not None and last_oom_kill is not None
                                                  else None),
                      "oom_kill_scope": "sampler cgroup only; G20 intentionally exercises OOM; absence does not rule out host or Podman OOM",
                      "scope": "tmpfs allocation only in tmpfs cell; entire filesystem in control"}), flush=True)


def cleanup(mode):
    common(mode)
    result = {"mode": mode, "status": "not-needed"}
    try:
        if not STATE.exists() or mode == "control":
            return
        owner = json.loads(STATE.read_text())
        assert owner["mode"] == mode and owner["graphroot"] == str(GRAPH)
        mounted = mount_info(True)
        if mounted is None:
            if owner["created_graphroot"] and GRAPH.is_dir():
                GRAPH.rmdir()  # refuses if another process wrote there
            result["status"] = "not-mounted"
            return
        assert mounted["fstype"] == "tmpfs" and mounted["source"] == owner["source"], "not own mount"
        try:
            running = json.loads(run("podman", "ps", "--format", "json", timeout=20))
            assert not running, "job still has running containers; refusing unmount"
            result["running_containers"] = 0
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, FileNotFoundError):
            # Unsupported storage can prevent inventory; a normal umount still
            # refuses any busy mount. Never force or lazily detach it.
            result["running_containers"] = "inventory-unavailable"
        run("sudo", "umount", str(GRAPH), timeout=30)
        assert mount_info(True) is None, "mount remained after umount"
        if owner["created_graphroot"]:
            GRAPH.rmdir()
        result["status"] = "unmounted"
    except BaseException as exc:
        result = {"mode": mode, "status": "failed", "error_type": type(exc).__name__,
                  "reason": str(exc)}
        raise
    finally:
        save("cleanup.json", result)


if __name__ == "__main__":
    task, selected = sys.argv[1:3]
    try:
        {"source": source, "prepare": prepare, "verify": verify,
         "sample": sample, "cleanup": cleanup,
         "final-source": lambda mode: source(mode, "final-source.json")}[task](selected)
    except BaseException as exc:
        save(f"{task}-failure.json", {"error_type": type(exc).__name__, "reason": str(exc)})
        raise
