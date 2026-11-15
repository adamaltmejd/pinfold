#!/usr/bin/env python3
"""Linux real-binary crash barriers: crash-windows.py BIN IMAGE [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import subprocess as sp
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
image = sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfcrash.", dir="/tmp")).resolve()
work.mkdir(exist_ok=True)
repo = Path(__file__).resolve().parents[3]
source = repo / "crates/pinfold/src/core/runtime/sockets.rs"
host = Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
prefix = f"probe-crash-{os.getpid()}"

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user"), flush=True)

def run(args, code=0, data=b""):
    result = sp.run([binary] + args, env=env, cwd=work, input=data, capture_output=True, timeout=180)
    emit("$ " + shlex.join(["$BIN"] + args))
    emit(f"exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
    assert result.returncode == code, result
    return result

def anchor(text, source_file=source):
    matches = [i for i, line in enumerate(source_file.read_text().splitlines(), 1) if text in line]
    assert len(matches) == 1, (text, matches)
    return matches[0]

def plan(name):
    return {"name": name, "image": image, "cpus": 2, "memory": "1G", "egress": {"allow": []}}

def start(name):
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    child.stdin.write(json.dumps(plan(name)).encode() + b"\n")
    child.stdin.flush()
    ready = json.loads(child.stdout.readline())
    assert ready["event"] == "ready", ready
    return child

def finish(name, child):
    run(["box", "down", name])
    out, err = child.communicate(timeout=180)
    emit(f"owner exit={child.returncode} stdout={out!r} stderr={err!r}")
    assert child.returncode == 0

def corrected_control(name):
    child = start(name)
    try:
        assert run(["box", "exec", name, "--", "printf", "CORRECTED-CONTROL"]).stdout == b"CORRECTED-CONTROL"
    finally:
        finish(name, child)

def debug(case, name, location, args=None, action="kill", source_file=source, program=binary):
    args = args or ["box", "up"]
    key = hashlib.sha256(f"box:{name}".encode()).hexdigest()
    record = host / (key + ".owner")
    spec = work / (case + ".json")
    spec.write_text(json.dumps(plan(name)) + "\n" if args == ["box", "up"] else "")
    observation = work / (case + "-observation.json")
    commands = work / (case + ".gdb")
    # GDB stops all threads at this source instruction. The observer then
    # kills the real owner, with no binary hook or replacement runtime.
    script = f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
set startup-with-shell on
python
import gdb, json, os, pathlib, signal, stat, subprocess, time
down = None
os.umask(0o022)
class Crash(gdb.Breakpoint):
    def stop(self):
        global down
        owner = pathlib.Path({str(record)!r})
        if not owner.exists():
            return False
        self.enabled = False
        observed = json.loads(owner.read_text())
        generation = observed["generation"]
        base = owner.parent
        paths = [base / ("r-" + generation[:32]), base / (".r-" + generation), base / (".d-" + generation), base / {key + ".sock"!r}]
        paths += [p for d in paths if d.is_dir() for p in d.rglob("*") if p.is_socket()]
        entries = []
        for path in paths:
            if path.exists():
                mode = path.lstat().st_mode
                entries.append({{"name": path.name, "mode": oct(stat.S_IMODE(mode)), "socket": stat.S_ISSOCK(mode), "children": sorted(p.name for p in path.iterdir()) if path.is_dir() else []}})
        result = {{"case": {case!r}, "source_line": {location}, "generation": generation, "pid": gdb.selected_inferior().pid, "entries": entries}}
        if {action!r} == "down":
            down = subprocess.Popen([{binary!r}, "box", "down", {name!r}], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            deadline = time.monotonic() + 7
            result["down_connected"] = False
            while time.monotonic() < deadline and down.poll() is None:
                where = pathlib.Path("/proc/" + str(down.pid) + "/wchan").read_text()
                if "unix" in where or "skb_wait" in where:
                    result["down_connected"] = True
                    result["down_wait"] = where
                    break
                os.sched_yield()
        if {action!r} in ("rename", "mount"):
            original = base / ("r-" + generation[:32])
            moved = base / ("probe-retained-" + generation)
            original.rename(moved)
            original.symlink_to({str(work / (case + "-target"))!r}, target_is_directory=True)
            result["moved"] = str(moved)
        pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
        gdb.write("EXTERNAL-BARRIER " + json.dumps(result) + "\\n")
        if {action!r} == "kill":
            os.kill(result["pid"], signal.SIGKILL)
            return True
        return False
class MountDone(gdb.Breakpoint):
    def stop(self):
        self.enabled = False
        result = json.loads(pathlib.Path({str(observation)!r}).read_text())
        children = set()
        for task in pathlib.Path('/proc/'+str(result['pid'])+'/task').iterdir():
            children.update(task.joinpath('children').read_text().split())
        vmms = [pid for pid in children if pathlib.Path('/proc/'+pid+'/comm').read_text().strip()=='firecracker']
        assert len(vmms)==1, vmms
        actual = pathlib.Path('/proc/'+vmms[0]+'/root/vsock').stat()
        expected = pathlib.Path(result['moved']).joinpath('v').stat()
        assert (actual.st_dev,actual.st_ino)==(expected.st_dev,expected.st_ino)
        original = pathlib.Path({str(host)!r})/('r-'+result['generation'][:32])
        original.unlink();pathlib.Path(result['moved']).rename(original)
        result.update(mount_identity=True, restored=True)
        pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
        return False
if {action!r} == 'mount':
    MountDone({str(repo/'crates/pinfold/src/core/runtime/firecracker/vm.rs') + ':' + str(anchor('super::record_vmm(&state_dir, vmm.pid)?;', repo/'crates/pinfold/src/core/runtime/firecracker/vm.rs'))!r})
Crash({str(source_file) + ":" + str(location)!r})
end
run {shlex.join(args)} < {shlex.quote(str(spec))} > {shlex.quote(str(work / (case + "-owner.out")))} 2> {shlex.quote(str(work / (case + "-owner.err")))}
{ 'continue' if action == 'kill' else '' }
python
if {action!r} != "kill":
    result = json.loads(pathlib.Path({str(observation)!r}).read_text())
    result["exit"] = int(gdb.parse_and_eval("$_exitcode"))
    if down is not None:
        out, err = down.communicate(timeout=30)
        result.update(down_exit=down.returncode, down_out=out.decode(), down_err=err.decode())
    pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
end
'''
    commands.write_text(script)
    result = sp.run(["gdb", "-nx", "-q", "-batch", "-x", str(commands), "--args", str(program)] + args,
                    env=env, cwd=repo, capture_output=True, timeout=240)
    emit(f"$ gdb -nx -q -batch -x $W/{case}.gdb --args $BIN {shlex.join(args)}; exit={result.returncode}")
    for line in (result.stdout + result.stderr).decode(errors="backslashreplace").splitlines():
        if "EXTERNAL-BARRIER" in line or "SIGKILL" in line or "Error" in line:
            emit(line)
    assert observation.exists(), result.stdout + result.stderr
    observed = json.loads(observation.read_text())
    return observed

def prune():
    listed = [json.loads(line) for line in run(["box", "list", "--label", "dev.pinfold.owner"]).stdout.splitlines()]
    assert not [item["name"] for item in listed if not item["owner_alive"] and not item["name"].startswith(prefix)], "unrelated orphan prevents host-wide prune"
    run(["box", "prune"])

def crash(case, location):
    name = prefix + "-" + case
    key = hashlib.sha256(f"box:{name}".encode()).hexdigest()
    record = host / (key + ".owner")
    observed = debug(case, name, location)
    if case == "bound-socket":
        endpoint = next(entry for entry in observed["entries"] if entry["name"] == key + ".sock")
        assert endpoint["socket"] and endpoint["mode"] == "0o600", endpoint
        assert all(entry["mode"] == "0o600" for entry in observed["entries"] if entry["socket"])
    prune()
    generation = observed["generation"]
    assert not record.exists()
    assert not (host / (key + ".sock")).exists()
    assert (host / (key + ".lock")).exists(), "stable name lock was unlinked"
    for leaf in ("r-" + generation[:32], ".r-" + generation, ".d-" + generation):
        assert not (host / leaf).exists(), leaf
    emit(f"{case}: public prune removed partial generation and claim; stable name lock retained")
    corrected_control(name)

# Anchors intentionally name real production instructions, not test hooks.
crash("before-marker", anchor('        let mut marker = open_at('))
crash("empty-marker", anchor('marker.write_all(generation.as_bytes())'))
crash("after-marker-unlink", anchor('unlink_at(host, leaf, libc::AT_REMOVEDIR)'))
# Set this at the owner-side return after the atomic bind helper is reaped.
crash("bound-socket", anchor('received.map_err(failure)?;'))

# Reproduce the eliminated bind/chmod window in the preserved 34bb76a binary.
legacy=Path('/tmp/pfrt-review-baseline')
name=prefix+'-legacy-wide-bind'
observed=debug('legacy-wide-bind',name,383,program=legacy)
assert any(entry['socket'] and entry['mode']!='0o600' for entry in observed['entries'])
prune();corrected_control(name)
emit('A legacy34bb76a killed after bind before chmod; wider socket recovered by fixed prune/start')

# A corrupt ordinary claim is a generic startup failure, not runtime-path.
name = prefix + "-corrupt-record"
record = host / (hashlib.sha256(f"box:{name}".encode()).hexdigest() + ".owner")
descriptor = os.open(record, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
os.write(descriptor, b"{")
os.close(descriptor)
try:
    refused = run(["box", "up"], code=1, data=json.dumps(plan(name)).encode() + b"\n")
    assert json.loads(refused.stdout)["event"] == "failed" and b"runtime-path" not in refused.stdout + refused.stderr
    assert record.read_bytes() == b"{"
finally:
    record.unlink()
emit("corrupt owner JSON stayed generic failed; fixture bytes unchanged")
corrected_control(name)

# Stop connects while generation publication is paused before VM preparation.
owner_source = repo / "crates/pinfold/src/core/ownership.rs"
name = prefix + "-published-down"
observed = debug("published-down", name, anchor('self.sockets = Some(Sockets::create(self.generation())?);', owner_source), action="down", source_file=owner_source)
emit("published-down=" + json.dumps(observed, sort_keys=True))
assert observed["down_connected"] and observed["down_exit"] == 0 and observed["exit"] == 0
assert b"ready" not in (work / "published-down-owner.out").read_bytes()
corrected_control(name)

# A retained O_PATH socket survives replacement of its named generation path.
name = prefix + "-retained-connect"
child = start(name)
target = work / "retained-connect-target"
target.mkdir(mode=0o700)
(target / "sentinel").write_bytes(b"UNCHANGED\n")
observed = None
try:
    observed = debug("retained-connect", name, anchor('std::os::unix::net::UnixStream::connect(format!("/proc/self/fd/{}", socket.as_raw_fd()))'),
                     args=["box", "exec", name, "--", "printf", "RETAINED-CONNECT"], action="rename")
    assert observed["exit"] == 0 and (work / "retained-connect-owner.out").read_bytes() == b"RETAINED-CONNECT"
    assert sorted(path.name for path in target.iterdir()) == ["sentinel"] and (target / "sentinel").read_bytes() == b"UNCHANGED\n"
    emit("retained descriptor connected to real guest after path replacement; target bytes and entries unchanged")
finally:
    if observed is None and (work / "retained-connect-observation.json").exists():
        observed = json.loads((work / "retained-connect-observation.json").read_text())
    if observed is not None and "moved" in observed:
        moved = Path(observed["moved"])
        original = host / ("r-" + observed["generation"][:32])
        original.unlink()
        moved.rename(original)
    finish(name, child)

# The inherited cwd pins the mounted inode even when its original path changes.
name=prefix+'-retained-mount'
target=work/'retained-mount-target';target.mkdir(mode=0o700);(target/'sentinel').write_bytes(b'UNCHANGED\n')
jail_source=repo/'crates/pinfold/src/core/runtime/firecracker/jail.rs'
observed=debug('retained-mount',name,anchor('nix::unistd::fchdir(vsock)',jail_source),action='mount',source_file=jail_source)
assert observed['exit']==0 and observed['mount_identity'] and observed['restored']
assert (target/'sentinel').read_bytes()==b'UNCHANGED\n' and len(list(target.iterdir()))==1
emit('B source-name replacement before namespace clone: VMM mounted retained inode; restored name booted and tore down; target unchanged')

# Choose the first actual record in readdir order as the unverifiable one.
pair=[]
for suffix in ('a','b'):
    name=prefix+'-continue-'+suffix
    observed=debug('continue-'+suffix,name,anchor('received.map_err(failure)?;'))
    record=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
    pair.append((record,observed))
order={p:i for i,p in enumerate(host.iterdir())}
pair.sort(key=lambda item:order[item[0]])
(bad_record,bad),(good_record,good)=pair
bad_dir=host/('r-'+bad['generation'][:32]);good_dir=host/('r-'+good['generation'][:32])
marker=bad_dir/'generation';marker.write_bytes(bad['generation'][:17].encode())
def snapshot_bad():
    return {str(p.relative_to(bad_dir)):(p.lstat().st_mode,p.read_bytes() if p.is_file() else None) for p in bad_dir.rglob('*')}
unchanged=snapshot_bad()
result=run(['box','prune'])
assert b'runtime-path' in result.stderr and not bad_record.exists() and not good_record.exists()
assert not good_dir.exists() and snapshot_bad()==unchanged
emit('A prune continued after first unverifiable dead record, retired both claims, removed later valid generation and preserved foreign bytes')
marker.write_bytes(bad['generation'].encode());prune();assert not bad_dir.exists()

# A malformed record cannot block cleanup of a later valid dead generation.
pair=[]
for suffix in ('a','b'):
    name=prefix+'-decode-'+suffix
    observed=debug('decode-'+suffix,name,anchor('received.map_err(failure)?;'))
    record=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
    pair.append((record,observed,name))
order={p:i for i,p in enumerate(host.iterdir())};pair.sort(key=lambda item:order[item[0]])
(bad_record,bad,bad_name),(good_record,good,good_name)=pair
original=bad_record.read_bytes();bad_record.write_bytes(b'{')
bad_dir=host/('r-'+bad['generation'][:32]);good_dir=host/('r-'+good['generation'][:32])
def decode_snapshot():
    return {str(p.relative_to(bad_dir)):(p.lstat().st_mode,p.read_bytes() if p.is_file() else None) for p in bad_dir.rglob('*')}
unchanged=decode_snapshot()
try:
    result=run(['box','prune'],code=1)
    assert b'EOF while parsing' in result.stderr and b'recordless' in result.stderr
    assert not good_record.exists() and not good_dir.exists()
    assert bad_record.read_bytes()==b'{' and decode_snapshot()==unchanged
    emit('A malformed first owner JSON reported; later valid dead claim pruned; corrupt record and its generation bytes preserved; record-less sweep stayed closed')
finally:
    bad_record.write_bytes(original)
prune();corrected_control(bad_name);corrected_control(good_name)

# Dry-run measurement and public prune never invoke shared image-cache cleanup.
def measured_bytes():
    report = run(["clean", "--dry-run"]).stdout.decode()
    matched = re.search(r"automatic maintenance: ([0-9]+) B(?:\n|$)", report)
    assert matched, "exact external byte comparison needs a byte-scale maintenance report"
    return int(matched[1])

before = measured_bytes()
fixtures, expected = [], 0
for index, (role, length) in enumerate(((".r-", None), (".r-", 17), (".d-", None), (".d-", 11), ("r-", 64))):
    generation = hashlib.sha256(f"{prefix}:leftover:{index}".encode()).hexdigest()
    directory = host / (role + (generation[:32] if role == "r-" else generation))
    directory.mkdir(mode=0o700)
    if length is not None:
        marker = directory / "generation"
        fd = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(fd, generation[:length].encode())
        os.close(fd)
        expected += length
    fixtures.append(directory)
foreign = host / ("r-" + hashlib.sha256(f"{prefix}:unverifiable".encode()).hexdigest()[:32])
foreign.mkdir(mode=0o700)
fd = os.open(foreign / "generation", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
os.write(fd, b"unverifiable-prefix")
os.close(fd)
try:
    after = measured_bytes()
    assert after - before == expected, (before, after, expected)
    prune()
    assert all(not directory.exists() for directory in fixtures)
    assert (foreign / "generation").read_bytes() == b"unverifiable-prefix"
    emit(f"recordless generation directories: measured delta={after-before} expected fixture bytes={expected}; full-role/marked leftovers swept; unverifiable prefix bytes preserved")
finally:
    (foreign / "generation").unlink()
    foreign.rmdir()
emit("PASS real crash windows, generic failure, early down, retained connect and leftover inventory/sweep")
