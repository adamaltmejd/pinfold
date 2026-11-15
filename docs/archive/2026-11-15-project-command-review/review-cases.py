#!/usr/bin/env python3
"""Real CLI cases: review-cases.py BIN IMAGE baseline|fixed [fresh-root]."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess as sp
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
image, phase = sys.argv[2:4]
fixed = phase == "fixed"
work = Path(sys.argv[4] if len(sys.argv) > 4 else tempfile.mkdtemp(prefix="pfreview.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G", EDITOR="true", VISUAL="true")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
projects = []

def emit(text):
    text = text.replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user" if sys.platform.startswith("linux") else "/Users/user")
    print(text.encode("utf-8", "backslashreplace").decode(), flush=True)

def run(args, cwd=None, code=None, data=b"", extra=None):
    result = sp.run([binary] + args, cwd=cwd or work, env=dict(env, **(extra or {})), input=data, capture_output=True, timeout=120)
    emit("$ " + shlex.join(["$BIN"] + args))
    emit(f"exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}")
    if code is not None:
        assert result.returncode == code
    return result

def git(root, *args):
    sp.run(["git", "-C", str(root), "-c", "user.name=user", "-c", "user.email=user@example.invalid", *args], check=True, capture_output=True)

def ident(root):
    return root.name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]

def project(name):
    root = work / name
    root.mkdir()
    git(root, "init", "-q")
    for part in ["plain", "sub", "internal"]:
        (root / part).mkdir()
        (root / part / "fixture").write_text("host\n")
    git(root, "add", ".")
    git(root, "commit", "-qm", "fixture")
    cfg = work / "c/pinfold/projects" / ident(root) / "pinfold.toml"
    cfg.parent.mkdir(parents=True)
    cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = ' + json.dumps(image) + '\n')
    projects.append(root)
    return root, cfg

root, cfg = project("probe-review")
emit("phase=" + phase + " work=" + str(work))
# 1a: the host directory exists but uncommitted contents never entered the disk.
missing = root / "untracked-cwd"
missing.mkdir()
r = run(["--", "sh", "-c", "echo CMD-STARTED"], missing, code=125 if fixed else None)
if fixed or sys.platform.startswith("linux"):
    assert b"CMD-STARTED" not in r.stdout
run(["--", "true"], root, code=0)
for code in [125, 126, 127]:
    run(["--", "sh", "-c", f"exit {code}"], root, code=code)
# 2: plant the symlink from within the real guest disk, then invoke that host cwd.
run(["--", "sh", "-c", "rm -rf sub; ln -s /home/pi sub"], root, code=0)
r = run(["--", "pwd"], root / "sub", code=125 if fixed else 0)
if not fixed:
    assert r.stdout == b"/home/pi\n"
run(["--", "sh", "-c", "rm sub; mkdir sub"], root, code=0)
r = run(["--", "pwd"], root / "sub", code=0)
assert r.stdout == (str(root / "sub") + "\n").encode()
run(["--", "sh", "-c", "rm -rf internal; ln -s plain internal"], root, code=0)
run(["--", "pwd"], root / "internal", code=125 if fixed else 0)
# 6: freshly formatted project roots must not retain formatter housekeeping.
r = run(["--", "sh", "-c", 'if test -e lost+found; then echo LOST-FOUND-PRESENT; else echo LOST-FOUND-ABSENT; fi'], root, code=0)
if fixed:
    assert b"LOST-FOUND-ABSENT" in r.stdout and b"lost+found" not in r.stderr
# 3: a missing share suffix still belongs to the profile's protected path list.
guard, guard_cfg = project("probe-share")
profile = work / "c/pinfold/profiles/probe-review"
profile.mkdir(parents=True)
(profile / "pinfold.toml").write_text("")
share = profile / "share"
share.symlink_to(guard / "missing")
guard_cfg.write_text(guard_cfg.read_text() + 'profile = "probe-review"\n')
r = run(["--", "true"], guard, code=125 if fixed else 0)
if fixed:
    assert b"host-path-in-project" in r.stderr
    assert not (work / "s/pinfold/projects" / ident(guard)).exists()
    emit("protected-share refusal left project state absent")
share.unlink()
outside = work / "outside-share"
outside.mkdir()
share.symlink_to(outside)
run(["--", "true"], guard, code=0)
# 4: argv is unchanged, but executable names in authored diagnostics are escaped.
r = run(["--", "/missing\npinfold: FORGED"], root, code=125 if fixed else 127)
if fixed:
    assert b"\npinfold: FORGED" not in r.stderr and b"\\npinfold: FORGED" in r.stderr
r = run(["--", "printf", "STREAM\\npinfold: direct-output\\n"], root, code=0)
assert r.stdout == b"STREAM\npinfold: direct-output\n"
sync_root, _ = project("probe-sync")
hook = "#!/bin/sh\nprintf 'SYNC-FIRST\\npinfold: FORGED\\033[31m\\n' >&2\nexit 1\n"
run(["--", "sh", "-c", 'printf "%s" "$1" > .git/hooks/reference-transaction; chmod +x .git/hooks/reference-transaction', "sh", hook], sync_root, code=0)
(sync_root / "new").write_text("new host commit\n")
git(sync_root, "add", "new")
git(sync_root, "commit", "-qm", "new fixture")
r = run(["--", "true"], sync_root, code=0)
assert b"sync-failed" in r.stderr
if fixed:
    assert b"\npinfold: FORGED" not in r.stderr and b"\\npinfold: FORGED" in r.stderr and b"\x1b" not in r.stderr
run(["--", "rm", ".git/hooks/reference-transaction"], sync_root, code=0)
# 1b and public process-interface controls. Readiness comes from the command.
def held():
    child = sp.Popen([binary, "--", "sh", "-c", "echo CMD-READY; read line; echo CMD-FINISHED"], cwd=root, env=env, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
    assert child.stdout.readline() == b"CMD-READY\n"
    listing = run(["box", "list", "--label", "dev.pinfold.project=" + ident(root)], root, code=0)
    names = [json.loads(line)["name"] for line in listing.stdout.splitlines()]
    assert len(names) == 1
    return child, names[0]

child, name = held()
for code in [125, 126, 127]:
    run(["box", "exec", name, "--", "sh", "-c", f"exit {code}"], root, code=code)
run(["box", "exec", name, "--", "/missing\npinfold: FORGED"], root, code=127)
bad = work / "not-git\npinfold: FORGED"
bad.mkdir()
r = run(["box", "fetch", name, "--repo", str(root), "--into", str(bad)], root, code=1)
detail = json.loads(r.stdout)["detail"]
if fixed:
    assert "\npinfold: FORGED" not in detail and "\\npinfold: FORGED" in detail
run(["box", "down", name], root, code=0)
out, err = child.communicate(timeout=120)
emit(f"external box down: project exit={child.returncode} stdout={out!r} stderr={err!r}")
assert child.returncode == (125 if fixed else 0) and b"CMD-FINISHED" not in out
r = run(["--", "sh", "-c", "echo CMD-READY; read line; echo CMD-FINISHED"], root, code=0, data=b"finish\n")
assert r.stdout == b"CMD-READY\nCMD-FINISHED\n"
child, _ = held()
child.send_signal(signal.SIGTERM)
out, err = child.communicate(timeout=120)
emit(f"host SIGTERM: project exit={child.returncode} stdout={out!r} stderr={err!r}")
assert child.returncode == 143
# 7: real bundled Pi parses the new delegate and follow-up shapes without login.
if sys.platform == "darwin":
    run(["--", "pi", "-p", "--version"], root, code=0)
    run(["--", "pi", "-p", "--continue", "--version"], root, code=0)
# Every disk here belongs to this fresh state root; no image is removed.
run(["disk", "rm", "--all", "--json"], code=0)
assert not run(["disk", "list", "--json"], code=0).stdout
emit("PASS review cases; fixtures=" + str(work))
