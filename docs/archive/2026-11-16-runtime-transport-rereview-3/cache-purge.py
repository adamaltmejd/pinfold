#!/usr/bin/env python3
"""Real Linux cache-purge crash race: cache-purge.py BIN PREPARED_IMAGE [root]."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import signal
import subprocess as sp
import sys
import tempfile
import time

assert sys.platform.startswith("linux") and os.geteuid() != 0
binary, image = str(Path(sys.argv[1]).resolve()), sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfcp.", dir="/tmp")).resolve()
work.mkdir(parents=True, exist_ok=True)
repo = Path(__file__).resolve().parents[3]
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
cache_home = Path(env.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
if not cache_home.is_absolute():
    cache_home = Path.home() / cache_home
name = "builder-" + hashlib.sha256(os.fsencode((cache_home / "pinfold").resolve())).hexdigest()[:32]
owner = Path("/tmp").resolve() / f"pinfold-{os.getuid()}" / (hashlib.sha256(f"box:{name}".encode()).hexdigest() + ".owner")
context = work / "context"
context.mkdir()
recipe = work / "Containerfile"
recipe.write_text(f"FROM {image}\nRUN printf cache-purge-control > /cache-purge-control\n")
image_name = f"probe-cache-purge-{os.getpid()}"
args = ["image", "build", image_name, "--containerfile", str(recipe), "--context", str(context)]

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user"), flush=True)

def run(argv, timeout=900):
    result = sp.run([binary] + argv, cwd=work, env=env, input=b"", capture_output=True, timeout=timeout)
    emit(f"$ $BIN {argv!r}; exit={result.returncode} stdout={result.stdout!r} stderr-tail={result.stderr[-1200:]!r}")
    assert result.returncode == 0, result
    return result

def location(source, anchor, within=None):
    lines = source.read_text().splitlines()
    start = next((i for i,line in enumerate(lines) if within and within in line), 0)
    end = next((i for i,line in enumerate(lines[start+1:], start+1) if within and (line.startswith("fn ") or line.startswith("async fn "))), len(lines))
    hits = [i for i, line in enumerate(source.read_text().splitlines(), 1) if anchor in line and start <= i-1 < end]
    assert len(hits) == 1, (anchor, hits)
    return f"{source}:{hits[0]}"

class Debugger:
    def __init__(self, label, argv, body, hold=False):
        self.gate = work / (label + ".gate")
        os.mkfifo(self.gate)
        self.buffer, self.history = b"", b""
        commands = work / (label + ".gdb")
        common = f'''import gdb, json, os, pathlib, signal
owner = pathlib.Path({str(owner)!r})
gate = {str(self.gate)!r}
def observe(phase):
    record = json.loads(owner.read_text())
    state = pathlib.Path(record["state_dir"])
    result = {{"phase":phase,"pid":gdb.selected_inferior().pid,"generation":record["generation"],"retiring":record.get("retiring",False),"state":str(state),"state_exists":state.exists(),"cache_exists":state.parent.exists(),"identity_exists":(state/"vmm").exists(),"owner_exists":owner.exists()}}
    gdb.write("CACHE-BARRIER " + json.dumps(result) + "\\n")
    gdb.flush()
    return result
'''
        redirection = f"< /dev/null > {shlex.quote(str(work / (label + '.out')))} 2> {shlex.quote(str(work / (label + '.err')))}"
        tail = f"python\nwith open({str(self.gate)!r}, 'rb') as release: release.read(1)\nend\n" if hold else ""
        commands.write_text("set pagination off\nset confirm off\nset print thread-events off\nset detach-on-fork on\nset follow-fork-mode parent\npython\n" + common + body + "\nend\nrun " + shlex.join(argv) + " " + redirection + "\n" + tail + "continue\n")
        self.process = sp.Popen(["gdb", "-nx", "-q", "-batch", "-x", str(commands), "--args", binary] + argv, cwd=repo, env=env, stdout=sp.PIPE, stderr=sp.STDOUT)
        emit(f"$ gdb -nx -q -batch -x $W/{label}.gdb --args $BIN {shlex.join(argv)}")
    def wait(self, phase):
        deadline = time.monotonic() + 900
        while True:
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                if line.startswith(b"CACHE-BARRIER "):
                    result = json.loads(line[len(b"CACHE-BARRIER "):])
                    emit("external " + json.dumps(result))
                    assert result["phase"] == phase, result
                    return result
            assert select.select([self.process.stdout], [], [], max(0, deadline-time.monotonic()))[0], f"no {phase} barrier"
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                emit("debugger tail=" + repr(self.history[-4000:]))
                raise AssertionError((phase, self.process.poll()))
            self.history = (self.history + chunk)[-65536:]
            self.buffer += chunk
    def release(self):
        with self.gate.open("wb") as gate:
            gate.write(b"x")
    def finish(self):
        output = self.process.communicate(timeout=60)[0]
        emit(f"debugger exit={self.process.returncode}; SIGKILL lines=" + repr([line for line in output.splitlines() if b"SIGKILL" in line]))

def locked():
    with owner.with_suffix(".lock").open("rb") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
    return False

builder = cleaner = None
builder_pid = owner_fd = None
try:
    source = repo / "crates/pinfold/src/core/image/linux/job.rs"
    builder = Debugger("builder", args, f'''class Live(gdb.Breakpoint):
    def stop(self):
        self.enabled = False
        observe("builder-live-after-spawn")
        return True
Live({location(source, 'let deadline = Instant::now() + JOB_TIME;', 'async fn run_job(')!r})''', hold=True)
    live = builder.wait("builder-live-after-spawn")
    builder_pid = live["pid"]
    owner_fd = os.pidfd_open(builder_pid)
    assert not live["retiring"] and live["identity_exists"] and locked(), live
    state = Path(live["state"])
    vmm = int((state / "vmm").read_text().split()[0])
    assert "firecracker" in Path(os.readlink(f"/proc/{vmm}/exe")).name
    pidfd = os.pidfd_open(vmm)
    assert not select.select([pidfd], [], [], 0)[0], "VMM already dead"
    source = repo / "crates/pinfold/src/core/image/linux.rs"
    cleaner = Debugger("cleaner", ["clean"], f'''class Cache(gdb.Breakpoint):
    def __init__(self, where, phase):
        super().__init__(where)
        self.phase = phase
    def stop(self):
        self.enabled = False
        result = observe(self.phase)
        if self.phase == "before-cache-lock":
            assert not result["retiring"] and result["identity_exists"]
            with open(gate, "rb") as release: release.read(1)
        elif self.phase == "retired-before-cache-mutation":
            assert result["retiring"] and result["identity_exists"] and result["state_exists"]
        else:
            assert result["retiring"] and not result["state_exists"] and result["owner_exists"]
            os.kill(result["pid"], signal.SIGKILL)
            return True
        return False
Cache({location(source, 'let Some(idle) = ownership::try_name_lock(&name)? else {')!r}, "before-cache-lock")
Cache({location(source, 'let cache = cache_dir()?;')!r}, "retired-before-cache-mutation")
Cache({location(source, 'ownership::remove_record(&name, &record.generation)?;')!r}, "cache-gone-owner-retained")''')
    before = cleaner.wait("before-cache-lock")
    assert before["generation"] == live["generation"] and not before["retiring"] and before["identity_exists"] and locked()
    signal.pidfd_send_signal(owner_fd, signal.SIGKILL)
    assert select.select([pidfd], [], [], 30)[0], "real VMM did not end"
    os.close(pidfd)
    builder.release()
    builder.finish()
    assert not locked()
    emit(f"clean passed orphan phases while builder lock held; killed real builder and observed VMM {vmm} death through pidfd")
    cleaner.release()
    retired = cleaner.wait("retired-before-cache-mutation")
    removed = cleaner.wait("cache-gone-owner-retained")
    assert retired["generation"] == removed["generation"] == live["generation"]
    assert retired["retiring"] and retired["state_exists"] and retired["identity_exists"]
    assert removed["retiring"] and not removed["state_exists"] and not removed["cache_exists"] and not removed["identity_exists"] and removed["owner_exists"]
    cleaner.finish()
    assert owner.exists() and json.loads(owner.read_text())["retiring"] and not state.exists()
    run(["box", "down", name])
    assert not owner.exists()
    built = run(args)
    assert json.loads(built.stdout)["event"] == "built" and not owner.exists()
    emit(f"PASS live-to-idle purge race, retirement before mutation, crash after cache deletion, targeted down and next real build; retained image={image_name}")
finally:
    if owner_fd is not None:
        try:
            signal.pidfd_send_signal(owner_fd, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(owner_fd)
    for debugger in (builder, cleaner):
        if debugger is not None and debugger.process.poll() is None:
            debugger.process.terminate()
            try:
                debugger.process.wait(timeout=15)
            except sp.TimeoutExpired:
                debugger.process.kill()
                debugger.process.wait()
