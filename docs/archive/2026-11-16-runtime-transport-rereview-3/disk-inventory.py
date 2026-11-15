#!/usr/bin/env python3
"""Linux real disk decision race: disk-inventory.py BIN IMAGE [fresh-root]."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import stat
import subprocess as sp
import sys
import tempfile

assert sys.platform.startswith("linux") and os.geteuid() != 0
binary, image = str(Path(sys.argv[1]).resolve()), sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfdi.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
repo = Path(__file__).resolve().parents[3]
name = f"probe-disk-inventory-{os.getpid()}"
disk = name + "-data"
host = Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
key = hashlib.sha256(f"box:{name}".encode()).hexdigest()
owner = host / (key + ".owner")
backing = work / "s/pinfold/disks" / (disk + ".ext4")
child = identity = saved_identity = None

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user"), flush=True)

def run(args, code=0):
    result = sp.run([binary] + args, cwd=work, env=env, input=b"", capture_output=True, timeout=180)
    emit(f"$ $BIN {args!r}; exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
    if code is not None:
        assert result.returncode == code, result
    return result

def snapshot():
    metadata = backing.stat()
    digest = hashlib.sha256()
    with backing.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            digest.update(chunk)
    return metadata.st_dev, metadata.st_ino, metadata.st_size, digest.hexdigest()

try:
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    child.stdin.write(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "disks": [{"name": disk, "guest": "/data", "size": "1G"}]}).encode() + b"\n")
    child.stdin.flush()
    ready = json.loads(child.stdout.readline())
    assert ready["event"] == "ready", ready
    assert run(["box", "exec", name, "--", "sh", "-c", "printf real-disk-fixture > /data/marker; sync; printf DISK-WRITTEN"]).stdout == b"DISK-WRITTEN"
    original = owner.read_bytes()
    record = json.loads(original)
    identity = Path(record["state_dir"]) / "vmm"
    saved_identity = identity.with_name("vmm.fixture-held")
    vmm = int(identity.read_text().split()[0])
    pidfd = os.pidfd_open(vmm)
    try:
        child.kill()
        child.wait(timeout=30)
        poll = select.poll()
        poll.register(pidfd, select.POLLIN)
        assert poll.poll(30000), "real VMM did not end after owner SIGKILL"
    finally:
        os.close(pidfd)
    for stream in (child.stdin, child.stdout, child.stderr):
        stream.close()
    assert child.returncode == -9
    with owner.with_suffix(".lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    emit(f"real owner SIGKILL exit=-9; VMM pid={vmm} pidfd readable; name lock independently acquired/released")
    before = snapshot()
    assert before[2] == 1 << 30
    identity.rename(saved_identity)
    source = repo / "crates/pinfold/src/core/disk.rs"
    locations = [i for i, line in enumerate(source.read_text().splitlines(), 1) if "let unstopped: BTreeSet<String> = runtime" in line]
    assert len(locations) == 1, locations
    observation, commands = work / "race.observed", work / "race.gdb"
    args = ["disk", "rm", disk, "--json"]
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
python
import gdb, json, pathlib, stat
class Race(gdb.Breakpoint):
    def stop(self):
        self.enabled = False
        owner = pathlib.Path({str(owner)!r})
        record = json.loads(owner.read_bytes())
        assert record["name"] == {name!r} and any(disk["name"] == {disk!r} for disk in record["disks"])
        result = {{"barrier": "attachments after first claim snapshot, before runtime decision inventory", "source_line": {locations[0]}, "frame": gdb.newest_frame().name(), "generation": record["generation"], "owner_inode": owner.stat().st_ino, "mode_before": oct(stat.S_IMODE(owner.stat().st_mode))}}
        owner.chmod(0)
        result["mode_after"] = oct(stat.S_IMODE(owner.stat().st_mode))
        pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
        gdb.write("EXTERNAL-DISK-BARRIER " + json.dumps(result) + "\\n")
        return False
Race({str(source) + ":" + str(locations[0])!r})
end
run {shlex.join(args)} < /dev/null > {shlex.quote(str(work / "race.out"))} 2> {shlex.quote(str(work / "race.err"))}
python
result = json.loads(pathlib.Path({str(observation)!r}).read_text())
result["exit"] = int(gdb.parse_and_eval("$_exitcode"))
pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
end
''')
    debug = sp.run(["gdb", "-nx", "-q", "-batch", "-x", str(commands), "--args", binary] + args, cwd=repo, env=env, capture_output=True, timeout=180)
    emit(f"$ gdb -nx -q -batch -x $W/race.gdb --args $BIN {shlex.join(args)}; exit={debug.returncode}")
    for line in (debug.stdout + debug.stderr).decode(errors="backslashreplace").splitlines():
        if "EXTERNAL-DISK-BARRIER" in line or "Error" in line:
            emit(line)
    assert observation.exists(), debug.stdout + debug.stderr
    observed = json.loads(observation.read_text())
    out, err = (work / "race.out").read_bytes(), (work / "race.err").read_bytes()
    emit(f"raced actual disk rm exit={observed['exit']} stdout={out!r} stderr={err!r}")
    assert observed["exit"] == 1 and err and b"runtime-path" not in out + err
    assert b"Permission denied" in out + err or b"inventory" in out + err or b"ownership record" in out + err
    assert owner.stat().st_ino == observed["owner_inode"] and stat.S_IMODE(owner.stat().st_mode) == 0
    assert snapshot() == before
    emit(f"raced inventory refusal preserved full 1GiB backing bytes, device/inode and size: {before!r}; owner record retained at mode000")
    for mutation in (["clean", "--unused", "0s"], ["box", "prune"]):
        refused = run(mutation, code=1)
        assert b"Permission denied" in refused.stderr + refused.stdout
        assert snapshot() == before and stat.S_IMODE(owner.stat().st_mode) == 0
    emit("unreadable claim also failed closed for clean and prune; backing bytes and claim mode unchanged")
    owner.chmod(0o600)
    assert owner.read_bytes() == original
    refused = run(args, code=1)
    assert b"unknown VMM" in refused.stdout + refused.stderr or b"disk-in-use" in refused.stdout + refused.stderr or b"inventory" in refused.stdout + refused.stderr
    assert owner.read_bytes() == original and snapshot() == before
    emit("readable owner JSON with unknown VMM identity also refused disk removal; full backing bytes/inode unchanged")
    for mutation in (["clean", "--unused", "0s"], ["box", "prune"]):
        refused = run(mutation, code=1)
        assert b"unknown VMM" in refused.stderr + refused.stdout
        assert snapshot() == before and owner.read_bytes() == original
    emit("unknown identity also failed closed for clean and prune; backing bytes and claim unchanged")
    displayed = run(["box", "list", "--label", "dev.pinfold.owner"])
    row = next(json.loads(line) for line in displayed.stdout.splitlines() if json.loads(line)["name"] == name)
    assert row["state"] == "unknown"
    healthy_name = name + "-healthy"
    healthy = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    healthy.stdin.write(json.dumps({"name":healthy_name,"image":image,"cpus":2,"memory":"1G"}).encode()+b"\n");healthy.stdin.flush()
    try:
        healthy_ready = json.loads(healthy.stdout.readline());assert healthy_ready["event"] == "ready",healthy_ready
        assert run(["box","exec",healthy_name,"--","printf","TARGET-LOCAL-CONTROL"]).stdout == b"TARGET-LOCAL-CONTROL"
        assert json.loads(run(["box","stat",healthy_name]).stdout)["memory"]["limit"] == 1 << 30
    finally:
        run(["box","down",healthy_name]);healthy.communicate(timeout=180)
    assert healthy.returncode == 0
    emit("display-only inventory reported unknown; unrelated targeted startup/exec/stat/down passed")
    boxes = Path(record["state_dir"]).parent
    held_boxes = work / "boxes.fixture-held"
    boxes.rename(held_boxes)
    try:
        refused = run(args, code=1)
        assert b"unknown VMM" in refused.stdout + refused.stderr
        assert snapshot() == before and owner.read_bytes() == original
        emit("missing whole boxes/state tree refused disk removal; full backing bytes/inode unchanged")
    finally:
        held_boxes.rename(boxes)
    saved_identity.rename(identity)
    displayed = run(["box", "list", "--label", "dev.pinfold.owner"])
    row = next(json.loads(line) for line in displayed.stdout.splitlines() if json.loads(line)["name"] == name)
    assert row["state"] == "stopped"
    emit("repaired complete identity is displayed as stopped; unknown output contract control passed")
    refused = run(args, code=1)
    assert b"disk-in-use" in refused.stdout + refused.stderr and snapshot() == before
    emit("known dead but unretired claim still pins its disk until retirement is published")
    run(["box", "down", name])
    assert not owner.exists()
    removed = run(args)
    assert json.loads(removed.stdout)["event"] == "removed" and not backing.exists()
    emit("PASS repaired identity permitted targeted retirement/down and the same real disk rm succeeded")
finally:
    if owner.exists():
        owner.chmod(0o600)
    if saved_identity is not None and saved_identity.exists() and not identity.exists():
        saved_identity.rename(identity)
    run(["box", "down", name], code=None)
    if child is not None and child.poll() is None:
        child.communicate(timeout=180)
    run(["disk", "rm", disk, "--json"], code=None)
