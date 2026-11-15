#!/usr/bin/env python3
"""Exercise the ordinary updater on a disposable, lower-version executable."""
import os
from pathlib import Path
import shutil
import subprocess as sp
import sys
import tempfile

source = Path(sys.argv[1]).resolve()
work = Path(tempfile.mkdtemp(prefix="pfupdate.", dir="/tmp")).resolve()
parent = work / "install\npinfold: FORGED"
parent.mkdir()
binary = parent / "pinfold"
shutil.copy2(source, binary)
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"), PINFOLD_NO_UPDATE_CHECK="1")

def emit(value):
    print(value.replace(str(work), "$W"), flush=True)

# The parent plants only this child's staging path before exec. No download
# or installation occurs in this negative control.
gate, release = os.pipe()
out, output = os.pipe()
err, errors = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(release)
    os.close(out)
    os.close(err)
    os.dup2(output, 1)
    os.dup2(errors, 2)
    os.read(gate, 1)
    os.execve(str(binary), [str(binary), "update"], env)
os.close(gate)
os.close(output)
os.close(errors)
staging = parent / f".pinfold-update-{pid}"
staging.write_text("fixture conflict\n")
os.write(release, b"x")
os.close(release)
stdout = os.fdopen(out, "rb").read()
stderr = os.fdopen(err, "rb").read()
_, status = os.waitpid(pid, 0)
code = os.waitstatus_to_exitcode(status)
emit("$ $W/'install\\npinfold: FORGED'/pinfold update; planted staging conflict")
emit(f"exit={code} stdout={stdout!r} stderr={stderr!r}")
assert code == 1 and b"cannot stage update beside" in stderr
assert b"\npinfold: FORGED" not in stderr and b"\\npinfold: FORGED" in stderr
staging.unlink()
# Positive control downloads/verifies the normal public release and replaces
# only this scratch executable. It does not touch the installed pinfold.
result = sp.run([str(binary), "update"], env=env, capture_output=True, timeout=180)
emit("$ $W/'install\\npinfold: FORGED'/pinfold update; conflict removed")
emit(f"exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
assert result.returncode == 0 and b"Updated pinfold" in result.stdout
assert b"\npinfold: FORGED" not in result.stdout and b"\\npinfold: FORGED" in result.stdout
version = sp.run([str(binary), "--version"], env=env, capture_output=True, check=True)
emit("replacement --version: " + repr(version.stdout))
assert b"0.0.0" not in version.stdout
emit("PASS updater path diagnostics; only disposable executable replaced")
