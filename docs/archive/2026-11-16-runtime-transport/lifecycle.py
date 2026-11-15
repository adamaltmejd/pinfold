#!/usr/bin/env python3
"""Real CLI acceptance: lifecycle.py BIN IMAGE [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import stat
import subprocess as sp
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
image = sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfrt.", dir="/tmp")).resolve()
linux = sys.platform.startswith("linux")
host = Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
state = work / ("long-state-" + "x" * 180)
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(state),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G",
           PINFOLD_TRANSPORT_FIXTURE="environment-delivered", EDITOR="true", VISUAL="true")
(state / "pinfold").mkdir(parents=True)
(state / "pinfold/maintenance").touch()
prefix = f"probe-transport-{os.getpid()}"
owners = {}

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()),
          "/home/user" if linux else "/Users/user"), flush=True)

def run(args, code=0, data=b"", cwd=None):
    result = sp.run([binary] + args, cwd=cwd or work, env=env, input=data,
                    capture_output=True, timeout=180)
    emit("$ " + shlex.join(["$BIN"] + args))
    output = repr(result.stdout) if len(result.stdout) < 800 else f"{len(result.stdout)} bytes sha256={hashlib.sha256(result.stdout).hexdigest()}"
    emit(f"exit={result.returncode} stdout={output} stderr={result.stderr!r}")
    assert result.returncode == code, result
    return result

def spec(name, disk=False):
    result = {"name": name, "image": image, "cpus": 2, "memory": "1G",
              "egress": {"allow": []}, "env": {"FIXTURE": {"from": "PINFOLD_TRANSPORT_FIXTURE"}}}
    if disk:
        result["disks"] = [{"name": prefix + "-data", "guest": "/data", "size": "1G"}]
    return result

def start(plan):
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work,
                     stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    child.stdin.write(json.dumps(plan).encode() + b"\n")
    child.stdin.flush()
    line = child.stdout.readline()
    ready = json.loads(line)
    emit("$ $BIN box up < " + json.dumps(plan, sort_keys=True))
    emit("first=" + line.decode().strip())
    assert ready["event"] == "ready", ready
    owners[plan["name"]] = child
    runtime = host / ("r-" + ready["run_id"][:32])
    assert runtime.is_dir() and (runtime / "generation").read_text().strip() == ready["run_id"]
    paths = [runtime] + list(runtime.rglob("*"))
    for path in paths:
        mode = stat.S_IMODE(path.stat().st_mode)
        assert mode == (0o700 if path.is_dir() else 0o600), (path, oct(mode))
    emit(f"state-root-bytes={len(os.fsencode(state))}; runtime={runtime}; modes=0700/0600")
    return child, ready, runtime

def finish(name, eof=False):
    child = owners.pop(name)
    if eof:
        child.stdin.close()
        child.stdin = None
    else:
        run(["box", "down", name])
    out, err = child.communicate(timeout=180)
    emit(f"owner exit={child.returncode} stdout={out!r} stderr={err!r}; eof={eof}")
    assert child.returncode == 0
    terminal = json.loads(out.splitlines()[-1])
    assert terminal["event"] == "down" and terminal["reason"] == ("stdin-closed" if eof else "signal")

def exec_(name, *args):
    return run(["box", "exec", name, "--", *args]).stdout

def checked_prune(code=0):
    listed = run(["box", "list", "--label", "dev.pinfold.owner"]).stdout.splitlines()
    others = [json.loads(line)["name"] for line in listed
              if not json.loads(line)["owner_alive"] and not json.loads(line)["name"].startswith(prefix)]
    assert not others, f"refusing host-wide prune with unrelated orphans: {others}"
    return run(["box", "prune"], code=code)

try:
    witness = prefix + "-live"
    _, live, live_dir = start(spec(witness))
    live_identity = live_dir.stat().st_ino
    name = prefix + "-disk"
    child, ready, runtime = start(spec(name, disk=True))
    assert exec_(name, "sh", "-c", 'printf "%s\\n" "$FIXTURE"; printf durable > /data/marker') == b"environment-delivered\n"
    run(["box", "stat", name])
    if linux:
        key = hashlib.sha256(f"box:{name}".encode()).hexdigest()
        record = json.loads((host / (key + ".owner")).read_text())
        pid = int((Path(record["state_dir"]) / "vmm").read_text().split()[0])
        jail = Path(f"/proc/{pid}/root")
        actual = sorted(path.name for path in (jail / "vsock").iterdir())
        assert set(actual) <= {"v.sock", "v.sock_1000", "v.sock_3128"} and "v.sock" in actual
        shutdown = host / (key + ".sock")
        assert shutdown.exists() and not (jail / shutdown.relative_to("/")).exists()
        emit(f"VMM pid={pid}; jail /vsock={actual}; shutdown={shutdown}; absent from jail")
    finish(name)
    assert not runtime.exists()
    child, ready, runtime = start(spec(name, disk=True))
    assert exec_(name, "cat", "/data/marker") == b"durable"
    child.kill()
    child.wait(timeout=30)
    # Apple's surviving native client retains these pipes until orphan removal.
    for stream in (child.stdin, child.stdout, child.stderr):
        stream.close()
    owners.pop(name)
    emit(f"owner SIGKILL exit={child.returncode}; host relay pipes closed before prune")
    assert child.returncode == -signal.SIGKILL
    # Only our orphan directory is changed; the shared host directory stays intact.
    saved = runtime.with_name(runtime.name + ".saved")
    target = work / "symlink-target"
    target.mkdir(mode=0o700)
    sentinel = target / "unchanged"
    sentinel.write_bytes(b"target bytes stay unchanged\n")
    before = sentinel.read_bytes()
    runtime.rename(saved)
    runtime.symlink_to(target, target_is_directory=True)
    try:
        # Apple must remove its native orphan before a same-name up can claim.
        # Socket cleanup then refuses, preserving the old claim and target.
        refused_prune = checked_prune(code=1)
        assert b"runtime-path" in refused_prune.stdout + refused_prune.stderr
        refused = run(["box", "up"], code=1, data=json.dumps(spec(name, disk=True)).encode() + b"\n")
        assert b"runtime-path" in refused.stdout + refused.stderr
        assert sentinel.read_bytes() == before and sorted(path.name for path in target.iterdir()) == ["unchanged"]
        emit("symlinked orphan runtime start refused runtime-path; target bytes and entries unchanged")
    finally:
        runtime.unlink()
        saved.rename(runtime)
    runtime.chmod(0o755)
    try:
        refused = run(["box", "up"], code=1, data=json.dumps(spec(name, disk=True)).encode() + b"\n")
        assert b"runtime-path" in refused.stdout + refused.stderr and stat.S_IMODE(runtime.stat().st_mode) == 0o755
        emit("unsafe orphan directory refused runtime-path; binary left mode 0755 unchanged")
    finally:
        runtime.chmod(0o700)
    checked_prune()
    assert live_dir.stat().st_ino == live_identity
    assert exec_(witness, "printf", "LIVE-SURVIVES") == b"LIVE-SURVIVES"
    emit("another live generation's runtime directory identity and exec survived prune")
    _, restarted, _ = start(spec(name, disk=True))
    assert restarted["run_id"] != ready["run_id"] and not runtime.exists()
    assert exec_(name, "cat", "/data/marker") == b"durable"
    finish(name)
    finish(witness, eof=True)
    if linux:
        source = work / "mount-source"
        source.mkdir()
        (source / "fixture").write_text("attachment-control\n")
        plan = spec(prefix + "-attachments")
        plan["mounts"] = [{"host": str(source), "guest": f"/m{i}"} for i in range(15)]
        refused = run(["box", "up"], code=1, data=json.dumps(plan).encode() + b"\n")
        assert b"spec" in refused.stdout and b"15" in refused.stdout
        plan["mounts"].pop()
        start(plan)
        assert exec_(plan["name"], "cat", "/m13/fixture") == b"attachment-control\n"
        finish(plan["name"])
        emit("Linux attachment 15 refused; corrected attachment 14 ran and read m13")
    # Reuse the original PR 1 CLI cases unchanged for argv/cwd/status/byte streams.
    original = Path(__file__).resolve().parent.parent / "2026-11-15-project-commands/cases.py"
    case_root = work / "commands"
    case_root.mkdir()
    result = sp.run([sys.executable, str(original), binary, image, str(case_root)], capture_output=True, timeout=1200)
    emit("$ python3 ../2026-11-15-project-commands/cases.py $BIN " + shlex.quote(image) + " $W/commands")
    for line in result.stdout.decode().splitlines():
        if line.startswith("binary I/O:") or line.startswith("PASS ") or "exit=" in line:
            emit(line)
    emit(f"PR1 driver exit={result.returncode}; stderr={result.stderr!r}")
    assert result.returncode == 0
    run(["disk", "rm", "--all", "--json"])
    command_env = dict(env, XDG_STATE_HOME=str(case_root / "s"))
    cleanup = sp.run([binary, "disk", "rm", "--all", "--json"], cwd=work, env=command_env, capture_output=True, timeout=180)
    emit(f"PR1 isolated disks removed exit={cleanup.returncode} stdout={cleanup.stdout!r} stderr={cleanup.stderr!r}")
    assert cleanup.returncode == 0
    emit("PASS long-root lifecycle, private sockets and compatibility controls")
finally:
    for name in list(owners):
        finish(name)
