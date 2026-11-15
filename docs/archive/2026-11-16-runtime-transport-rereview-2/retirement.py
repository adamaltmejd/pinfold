#!/usr/bin/env python3
"""Linux real retirement barriers: retirement.py BIN PREPARED_IMAGE [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess as sp
import sys
import tempfile

binary, image = str(Path(sys.argv[1]).resolve()), sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfret.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
repo = Path(__file__).resolve().parents[3]
host = Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
cache_home = Path(env.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
cache_home = cache_home if cache_home.is_absolute() else Path.home() / ".cache"
builder = "builder-" + hashlib.sha256(os.fsencode((cache_home / "pinfold").resolve())).hexdigest()[:32]
prefix = f"probe-retirement-{os.getpid()}"

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user"), flush=True)

def run(args, timeout=600):
    result = sp.run([binary] + args, cwd=work, env=env, input=b"", capture_output=True, timeout=timeout)
    emit(f"$ $BIN {args!r}; exit={result.returncode} stdout={result.stdout!r} stderr-tail={result.stderr[-1200:]!r}")
    assert result.returncode == 0, result
    return result

def control(name):
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    child.stdin.write(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "egress": {"allow": []}}).encode() + b"\n")
    child.stdin.flush()
    ready = json.loads(child.stdout.readline())
    assert ready["event"] == "ready", ready
    try:
        assert run(["box", "exec", name, "--", "printf", "RETIREMENT-CONTROL"]).stdout == b"RETIREMENT-CONTROL"
    finally:
        run(["box", "down", name])
        out, err = child.communicate(timeout=180)
        emit(f"corrected owner exit={child.returncode} stdout={out!r} stderr={err!r}")
        assert child.returncode == 0

def barrier(case, name, source, anchor, state_expected, args=None, after_return=False):
    args = args or ["box", "up"]
    matches = [i for i, line in enumerate(source.read_text().splitlines(), 1) if anchor in line]
    assert len(matches) == 1, (anchor, matches)
    key = hashlib.sha256(f"box:{name}".encode()).hexdigest()
    owner = host / (key + ".owner")
    spec = work / (case + ".stdin")
    spec.write_text(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "egress": {"allow": []}}) + "\n" if args == ["box", "up"] else "")
    observation, commands = work / (case + ".observed"), work / (case + ".gdb")
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb, json, os, pathlib, signal
saved = None
def observe(label):
    global saved
    owner = pathlib.Path({str(owner)!r})
    if owner.exists():
        saved = json.loads(owner.read_text())
    result = {{"case": {case!r}, "barrier": label, "pid": gdb.selected_inferior().pid, "generation": saved["generation"], "retiring": saved.get("retiring", False), "owner_exists": owner.exists(), "state_dir": saved["state_dir"], "state_exists": pathlib.Path(saved["state_dir"]).exists()}}
    pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
    gdb.write("EXTERNAL-RETIREMENT " + json.dumps(result) + "\\n")
    return result
class Returned(gdb.FinishBreakpoint):
    def stop(self):
        result = observe("after-owner-unlink-return")
        os.kill(result["pid"], signal.SIGKILL)
        return True
class Barrier(gdb.Breakpoint):
    def stop(self):
        self.enabled = False
        result = observe("before-source-instruction")
        if {after_return!r}:
            Returned(gdb.newest_frame(), internal=True)
            return False
        os.kill(result["pid"], signal.SIGKILL)
        return True
Barrier({str(source) + ":" + str(matches[0])!r})
end
run {shlex.join(args)} < {shlex.quote(str(spec))} > {shlex.quote(str(work / (case + ".out")))} 2> {shlex.quote(str(work / (case + ".err")))}
continue
''')
    result = sp.run(["gdb", "-nx", "-q", "-batch", "-x", str(commands), "--args", binary] + args,
                    env=env, cwd=repo, capture_output=True, timeout=900)
    emit(f"$ gdb -nx -q -batch -x $W/{case}.gdb --args $BIN {shlex.join(args)}; exit={result.returncode}")
    for line in (result.stdout + result.stderr).decode(errors="backslashreplace").splitlines():
        if "EXTERNAL-RETIREMENT" in line or "SIGKILL" in line or "Error" in line:
            emit(line)
    assert observation.exists(), result.stdout + result.stderr
    observed = json.loads(observation.read_text())
    assert observed["retiring"] and observed["state_exists"] == state_expected, observed
    assert observed["owner_exists"] != after_return, observed
    return observed, owner

def gone(observed, owner):
    generation = observed["generation"]
    assert not owner.exists()
    assert not Path(observed["state_dir"]).exists()
    for leaf in ("r-" + generation[:32], ".r-" + generation, ".d-" + generation):
        assert not (host / leaf).exists(), leaf
    assert not owner.with_suffix(".sock").exists()
    assert owner.with_suffix(".lock").exists(), "stable lock was removed"

ownership = repo / "crates/pinfold/src/core/ownership.rs"
unlink = 'fs::remove_file(path(&format!("box:{name}"), "owner")?)'
for case, anchor, state_expected, after_return, recovery in (
    ("box-before-delete", "Sockets::remove(generation)", True, False, "down"),
    ("box-state-gone", unlink, False, False, "prune"),
    ("box-owner-gone", unlink, False, True, "up"),
):
    name = prefix + "-" + case
    observed, owner = barrier(case, name, ownership, anchor, state_expected, after_return=after_return)
    if recovery == "prune":
        listed = [json.loads(line) for line in run(["box", "list", "--label", "dev.pinfold.owner"]).stdout.splitlines()]
        assert not [item["name"] for item in listed if not item["owner_alive"] and not item["name"].startswith(prefix)], "unrelated orphan prevents public prune"
        run(["box", "prune"])
    elif recovery == "down":
        run(["box", "down", name])
    control(name)
    gone(observed, owner)
    emit(f"{case}: {recovery} recovery and same-name real exec/down succeeded; prior claim, state and sockets absent")

context = work / "context"
context.mkdir()
recipe = work / "Containerfile"
recipe.write_text(f"FROM {image}\nRUN printf retirement-builder > /retirement-builder\n")
job = repo / "crates/pinfold/src/core/image/linux/job.rs"
for case, anchor, state_expected in (
    ("builder-before-delete", "let writable = tokio::task::spawn_blocking", True),
    ("builder-state-gone", "let cleanup = claim.finish(", False),
):
    image_name = prefix + "-" + case
    args = ["image", "build", image_name, "--containerfile", str(recipe), "--context", str(context)]
    observed, owner = barrier(case, builder, job, anchor, state_expected, args=args)
    if not state_expected:
        run(["box", "down", builder])
    built = run(args, timeout=900)
    assert json.loads(built.stdout)["event"] == "built", built.stdout
    gone(observed, owner)
    emit(f"{case}: next real image build succeeded; prior builder claim absent; retained image name={image_name}")
emit("PASS retirement publication, state-deletion and owner-unlink crash boundaries on real boxes/builders")
