#!/usr/bin/env python3
"""Real startup cancellation: startup-control.py BIN IMAGE baseline|fixed [root]."""
import hashlib
import json
import os
from pathlib import Path
import subprocess as sp
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
image, phase = sys.argv[2:4]
work = Path(sys.argv[4] if len(sys.argv) > 4 else tempfile.mkdtemp(prefix="pfreviewstart.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
root = work / "probe-startup"
root.mkdir()
sp.run(["git", "init", "-q", str(root)], check=True)
ident = root.name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg = work / "c/pinfold/projects" / ident / "pinfold.toml"
cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = ' + json.dumps(image) + '\n')
child = sp.Popen([binary, "--", "printf", "CMD-MUST-NOT-START"], cwd=root, env=env, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
name = f"pi-{ident}-{child.pid}"
record = Path("/tmp").resolve() / f"pinfold-{os.getuid()}" / (hashlib.sha256(("box:" + name).encode()).hexdigest() + ".owner")
deadline = time.monotonic() + 60
try:
    while not record.exists():
        assert child.poll() is None, "project exited before claiming its box"
        assert time.monotonic() < deadline, "no owner record before deadline"
    owned = json.loads(record.read_bytes())
    run = work / "s/pinfold/runs" / owned["generation"]
    assert not (run / "launch.json").exists(), "box was ready before cancellation"
    down = sp.run([binary, "box", "down", name], cwd=root, env=env, capture_output=True, timeout=60)
    output, errors = child.communicate(timeout=60)
    assert down.returncode == 0 and b"CMD-MUST-NOT-START" not in output
    assert child.returncode == (125 if phase == "fixed" else 0)
    assert not (run / "terminal.json").exists(), "cancellation happened after startup"
    print("$ $BIN -- printf CMD-MUST-NOT-START; wait for own owner record; assert launch absent; $BIN box down OWN-NAME", flush=True)
    print(f"phase={phase} owner-generation={owned['generation']} launch-before=false terminal-after=false down-exit={down.returncode} project-exit={child.returncode} stdout={output!r} stderr={errors!r}".replace(str(work), "$W").replace(binary, "$BIN"), flush=True)
finally:
    if child.poll() is None:
        child.terminate()
        child.communicate(timeout=60)
    cleaned = sp.run([binary, "disk", "rm", "--all", "--json"], cwd=root, env=env, capture_output=True, timeout=60)
    assert cleaned.returncode == 0
    print("own isolated disks removed; image retained", flush=True)
