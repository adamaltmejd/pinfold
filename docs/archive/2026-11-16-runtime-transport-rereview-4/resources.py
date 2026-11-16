#!/usr/bin/env python3
"""Linux real resource isolation: resources.py BIN IMAGE [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shlex
import stat
import subprocess as sp
import sys
import tempfile

assert sys.platform.startswith("linux") and os.geteuid() != 0
binary, image = str(Path(sys.argv[1]).resolve()), sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfrsrc.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
repo, host = Path(__file__).resolve().parents[3], Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
cache = Path(env.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
cache = (cache if cache.is_absolute() else Path.home() / ".cache") / "pinfold"
builder = "builder-" + hashlib.sha256(os.fsencode(cache.resolve())).hexdigest()[:32]
prefix = f"probe-resources-{os.getpid()}"
a, b, c = [prefix + suffix for suffix in ("-a", "-b", "-c")]
disks, owners = [prefix + suffix for suffix in ("-protected", "-one", "-two")], {}
builder_bytes = identity = a_bytes = None

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user"), flush=True)

def run(args, code=0):
    result = sp.run([binary] + args, cwd=work, env=env, input=b"", capture_output=True, timeout=240)
    emit(f"$ $BIN {args!r}; exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
    if code is not None: assert result.returncode == code, result
    return result

def record(name):
    return host / (hashlib.sha256(f"box:{name}".encode()).hexdigest() + ".owner")

def backing(disk):
    return work / "s/pinfold/disks" / (disk + ".ext4")

def snapshot(path):
    metadata, digest = path.stat(), hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""): digest.update(chunk)
    return metadata.st_ino, metadata.st_size, digest.hexdigest()

def start(name, attached=()):
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    owners[name] = child
    child.stdin.write(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "disks": [{"name": disk, "guest": f"/d{i}", "size": "1G"} for i, disk in enumerate(attached)]}).encode() + b"\n")
    child.stdin.flush()
    ready = json.loads(child.stdout.readline())
    emit(f"real box ready name={name} response={ready!r}")
    assert ready["event"] == "ready", ready

def dead(name):
    observed = json.loads(record(name).read_bytes())
    pid = int((Path(observed["state_dir"]) / "vmm").read_text().split()[0])
    fd, child = os.pidfd_open(pid), owners[name]
    child.kill(); child.wait(timeout=30)
    poll = select.poll(); poll.register(fd, select.POLLIN)
    try: assert poll.poll(30000), "VMM death readiness deadline"
    finally: os.close(fd)
    for stream in (child.stdin, child.stdout, child.stderr): stream.close()
    assert child.returncode == -9
    emit(f"real owner {name} SIGKILL=-9; real VMM pid={pid} pidfd readable")
    return observed

def reported(result):
    found = re.search(rb"  disks: ([0-9.]+) (B|KiB|MiB|GiB|TiB)", result.stdout)
    assert found, result.stdout
    unit = 1024 ** [b"B", b"KiB", b"MiB", b"GiB", b"TiB"].index(found[2])
    expected = sum(backing(disk).stat().st_blocks * 512 for disk in disks if backing(disk).exists())
    assert abs(float(found[1]) * unit - expected) <= 0.051 * unit, (found[0], expected)
    emit(f"reported disk allocation agrees with outside host st_blocks total={expected} bytes")

try:
    start(a, disks[:1]); run(["box", "exec", a, "--", "sh", "-c", "printf protected > /d0/marker; sync"])
    a_record = dead(a); a_bytes = record(a).read_bytes()
    identity = Path(a_record["state_dir"]) / "vmm"; identity.chmod(0)
    protected = snapshot(backing(disks[0]))
    context = work / "context"; context.mkdir()
    recipe = work / "Containerfile"; recipe.write_text(f"FROM {image}\nRUN true\n")
    source = repo / "crates/pinfold/src/core/ownership.rs"
    locations = [i for i, line in enumerate(source.read_text().splitlines(), 1) if "self.sockets = Some(Sockets::create(self.generation())?);" in line]
    assert len(locations) == 1, locations
    observation, commands = work / "builder.observed", work / "builder.gdb"
    args = ["image", "build", prefix + "-builder", "--containerfile", str(recipe), "--context", str(context)]
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb, json, os, pathlib, signal
class Published(gdb.Breakpoint):
    def stop(self):
        self.enabled = False
        observed = json.loads(pathlib.Path({str(record(builder))!r}).read_bytes())
        assert observed["name"] == {builder!r} and observed["disks"] == []
        assert (pathlib.Path(observed["state_dir"]) / "vmm").read_text() == "not-started\\n"
        pathlib.Path({str(observation)!r}).write_text(json.dumps(observed))
        gdb.write("REAL-DISKLESS-BUILDER-PUBLISHED " + observed["generation"] + " before VMM preparation\\n")
        os.kill(gdb.selected_inferior().pid, signal.SIGKILL)
        return True
Published({str(source) + ":" + str(locations[0])!r})
end
run {shlex.join(args)} < /dev/null > {shlex.quote(str(work / "builder.out"))} 2> {shlex.quote(str(work / "builder.err"))}
continue
''')
    result = sp.run(["gdb", "-nx", "-q", "-batch", "-x", str(commands), "--args", binary] + args, cwd=repo, env=env, capture_output=True, timeout=180)
    emit(f"$ gdb -nx -q -batch -x $W/builder.gdb --args $BIN {shlex.join(args)}; exit={result.returncode}")
    for line in (result.stdout + result.stderr).decode(errors="backslashreplace").splitlines():
        if "REAL-DISKLESS" in line or "SIGKILL" in line or "Error" in line: emit(line)
    assert observation.exists(), result.stdout + result.stderr
    builder_bytes = record(builder).read_bytes(); builder_record = json.loads(builder_bytes)
    record(builder).write_bytes(b"{")
    assert all(stat.S_IMODE(record(name).stat().st_mode) == 0o600 and os.access(record(name), os.R_OK) for name in (a, builder))
    start(b, disks[1:])
    assert run(["box", "exec", b, "--", "sh", "-c", "printf ONE >/d0/marker; printf TWO >/d1/marker; sync; cat /d0/marker /d1/marker"]).stdout == b"ONETWO"
    listed = [json.loads(line) for line in run(["disk", "list", "--json"]).stdout.splitlines()]
    assert {entry["name"] for entry in listed} == set(disks) and all(entry["size"] == 1 << 30 for entry in listed)
    reported(run(["doctor"])); reported(run(["clean", "--dry-run"]))
    run(["box", "down", b]); out, err = owners[b].communicate(timeout=180)
    assert owners[b].returncode == 0
    record(builder).write_bytes(builder_bytes)
    run(["disk", "rm", disks[0], "--json"], code=1)
    assert snapshot(backing(disks[0])) == protected
    emit("readable unknown-liveness A pins its own named disk; full backing bytes unchanged")
    run(["disk", "rm", disks[1], "--json"]); assert not backing(disks[1]).exists()
    emit("unrelated real disk removed with every owner JSON readable, despite A's unknown VMM identity")
    record(builder).write_bytes(b"{"); unrelated = snapshot(backing(disks[2]))
    assert stat.S_IMODE(record(builder).stat().st_mode) == 0o600 and os.access(record(builder), os.R_OK)
    run(["disk", "rm", disks[2], "--json"], code=1)
    assert snapshot(backing(disks[2])) == unrelated
    emit(f"readable malformed builder JSON refused unrelated disk removal; full backing bytes/inode preserved={unrelated!r}")
    start(c); c_record = dead(c)
    assert not [path for path in host.glob("*.owner") if path not in {record(a), record(builder), record(c)}], "foreign claim prevents actual clean"
    cleaned = run(["clean"], code=None); reported(cleaned)
    assert not record(c).exists() and not Path(c_record["state_dir"]).exists()
    assert not (host / ("r-" + c_record["generation"][:32])).exists()
    assert record(a).read_bytes() == a_bytes and stat.S_IMODE(identity.stat().st_mode) == 0
    assert record(builder).read_bytes() == b"{" and Path(builder_record["state_dir"]).exists()
    assert snapshot(backing(disks[0])) == protected and snapshot(backing(disks[2])) == unrelated and cleaned.stderr
    emit("actual clean pruned verified-dead C; A's unknown claim, real malformed builder claim and both disk byte snapshots survived; warnings and sizes retained")
    identity.chmod(0o600); record(builder).write_bytes(builder_bytes)
    run(["box", "down", a]); run(["box", "down", builder])
    for disk in (disks[0], disks[2]): run(["disk", "rm", disk, "--json"])
    emit("PASS resource-local startup, metadata reports, disk removal and independent cleanup")
finally:
    if identity is not None and identity.exists(): identity.chmod(0o600)
    if builder_bytes is not None and record(builder).exists(): record(builder).write_bytes(builder_bytes)
    for name in (a, b, c, builder):
        run(["box", "down", name], code=None)
        if name in owners and owners[name].poll() is None: owners[name].communicate(timeout=180)
    for disk in disks: run(["disk", "rm", disk, "--json"], code=None)
