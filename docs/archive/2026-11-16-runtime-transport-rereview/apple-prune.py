#!/usr/bin/env python3
"""Real Apple orphan isolation: apple-prune.py BIN IMAGE [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import subprocess as sp
import sys
import tempfile

binary, image = str(Path(sys.argv[1]).resolve()), sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfap.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
host = Path("/tmp").resolve() / f"pinfold-{os.getuid()}"
prefix = f"probe-apple-prune-{os.getpid()}"
owners, facts, bad, original = {}, {}, None, None

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/Users/user"), flush=True)

def run(args, code=0):
    result = sp.run([binary] + args, env=env, cwd=work, input=b"", capture_output=True, timeout=180)
    emit(f"$ $BIN {args!r}; exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
    if code is not None:
        assert result.returncode == code, result
    return result

def native():
    result = sp.run(["container", "list", "--all", "--format", "json"], capture_output=True, timeout=60)
    assert result.returncode == 0, result.stderr
    return [item["id"] for item in json.loads(result.stdout)]

def start(name):
    child = sp.Popen([binary, "box", "up"], env=env, cwd=work, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    owners[name] = child
    child.stdin.write(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "egress": {"allow": []}}).encode() + b"\n")
    child.stdin.flush()
    ready = json.loads(child.stdout.readline())
    assert ready["event"] == "ready", ready
    record = host / (hashlib.sha256(f"box:{name}".encode()).hexdigest() + ".owner")
    facts[name] = (record, host / ("r-" + ready["run_id"][:32]), Path(json.loads(record.read_bytes())["state_dir"]))
    emit(f"real owner ready name={name} pid={child.pid} generation={ready['run_id']}")
    return child

def safe_prune():
    listed = [json.loads(line) for line in run(["box", "list", "--label", "dev.pinfold.owner"]).stdout.splitlines()]
    assert not [item["name"] for item in listed if not item["owner_alive"] and item["name"] not in facts], "unrelated orphan prevents host-wide prune"
    return run(["box", "prune"])

try:
    for suffix in ("one", "two"):
        start(prefix + "-" + suffix)
    for name, child in owners.items():
        child.kill()
        child.wait(timeout=30)
        for stream in (child.stdin, child.stdout, child.stderr):
            stream.close()
        emit(f"owner SIGKILL name={name} exit={child.returncode}; native pipes closed without waiting for orphan EOF")
        assert child.returncode == -9
    actual = native()
    own_order = [name for name in actual if name in facts]
    assert len(own_order) == 2, actual
    emit(f"native container list before corruption: both real orphans present, iteration order={own_order!r}")
    listed = [json.loads(line) for line in run(["box", "list", "--label", "dev.pinfold.owner"]).stdout.splitlines()]
    assert not [item["name"] for item in listed if not item["owner_alive"] and item["name"] not in facts], "unrelated orphan prevents host-wide prune"
    bad_name, good_name = own_order
    bad, bad_dir, _ = facts[bad_name]
    good, good_dir, good_state = facts[good_name]
    original = bad.read_bytes()
    marker = (bad_dir / "generation").read_bytes()
    bad.write_bytes(b"{")
    result = run(["box", "prune"], code=None)
    actual = native()
    assert bad_name in actual and good_name not in actual, actual
    assert bad.read_bytes() == b"{" and (bad_dir / "generation").read_bytes() == marker
    assert not good.exists() and not good_dir.exists() and not good_state.exists()
    assert bad_name.encode() in result.stderr or bad.name.encode() in result.stderr, result.stderr
    emit("public prune diagnosed the first malformed owner, preserved its real container and marker bytes, and removed the later valid real orphan, generation and state")
    bad.write_bytes(original)
    safe_prune()
    assert not any(name in native() for name in facts)
    assert all(not record.exists() and not directory.exists() for record, directory, _ in facts.values())
    emit("repaired owner JSON; public prune removed both fixture containers and generation directories")
    child = start(bad_name)
    assert run(["box", "exec", bad_name, "--", "printf", "CORRECTED-CONTROL"]).stdout == b"CORRECTED-CONTROL"
    child.stdin.close()
    child.stdin = None
    out, err = child.communicate(timeout=180)
    emit(f"corrected owner EOF exit={child.returncode} stdout={out!r} stderr={err!r}")
    assert child.returncode == 0 and json.loads(out.splitlines()[-1])["reason"] == "stdin-closed"
    emit("PASS Apple real orphan corruption isolation, repair, corrected exec and EOF teardown")
finally:
    if bad is not None and original is not None and bad.exists() and bad.read_bytes() == b"{":
        bad.write_bytes(original)
    # Cleanup is targeted even when an assertion prevented the public-prune case.
    for name, child in owners.items():
        run(["box", "down", name], code=None)
        if child.poll() is None:
            child.communicate(timeout=180)
