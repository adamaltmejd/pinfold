#!/usr/bin/env python3
"""Real-binary builder cases. Usage: cases.py BINARY IMAGE [existing-root]."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess as sp
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
image = sys.argv[2]
work = Path(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="pfcmd.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G", EDITOR="true", VISUAL="true")
linux = sys.platform.startswith("linux")

def scrub(text):
    return text.replace(str(work), "$W").replace(str(Path.home()), "/home/user" if linux else "/Users/user").replace(binary, "$BIN")

def emit(text):
    print(scrub(text), flush=True)

def call(args, cwd=None, expect=0, data=b"", extra=None, stdout=None, err=None):
    merged = dict(env, **(extra or {}))
    result = sp.run([binary] + args, cwd=cwd or work, env=merged, input=data, capture_output=True, timeout=120)
    emit("$ " + shlex.join(["$BIN"] + args))
    output = repr(result.stdout) if len(result.stdout) < 512 else f"{len(result.stdout)} bytes sha256={hashlib.sha256(result.stdout).hexdigest()}"
    emit(f"exit={result.returncode} stdout={output} stderr={result.stderr!r}")
    assert result.returncode == expect, (args, result.returncode)
    if stdout is not None:
        assert result.stdout == stdout, result.stdout
    if err is not None:
        assert err in result.stderr, result.stderr
    return result

def project(name, git=True):
    root = work / name
    root.mkdir(exist_ok=True)
    if git:
        sp.run(["git", "init", "-q", str(root)], check=True)
        (root / "nested").mkdir(exist_ok=True)
        (root / "nested" / "fixture").write_text("host-committed\n")
        sp.run(["git", "-C", str(root), "add", "."], check=True)
        sp.run(["git", "-C", str(root), "-c", "user.name=user", "-c", "user.email=user@example.invalid",
                "commit", "-qm", "fixture"], check=True)
    return root

def identity(root):
    name = "".join(c if c.isascii() and (c.isalnum() or c in "-_.") else "-" for c in root.name)[:32]
    return name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]

def config(root):
    call(["config", "edit"], root)
    path = work / "c/pinfold/projects" / identity(root) / "pinfold.toml"
    path.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = ' + json.dumps(image) + '\n')
    return path

def no_state(root):
    assert not (work / "s/pinfold/projects" / identity(root)).exists()
    emit("project state absent")

# Bare help returns before executable locks, maintenance, or config/state creation.
result = sp.run([binary], cwd=work, env=env, capture_output=True)
assert result.returncode == 0 and b"pinfold -- CMD" in result.stdout
assert not (work / "s").exists() and not (work / "c").exists()
emit("$ $BIN; exit=0; help printed; config/state roots absent")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
root = project("probe-commands")
call(["--", "true"], root, expect=125, err=b"no-config")
no_state(root)
for args in [["--"], ["--", ""]]:
    call(args, root, expect=125, err=b"nonempty command")
no_state(root)
cfg = config(root)
call(["--", "true"], root, stdout=b"")
for code in [0, 23, 125, 126, 127]:
    call(["--", "sh", "-c", f"exit {code}"], root, expect=code, stdout=b"")
call(["--", "sh", "-c", "kill -TERM $$"], root, expect=143, stdout=b"")
call(["--", "printf", "<%s>\\n", "", "a b", "$(touch /tmp/should-not-exist);*", "--help", "--"], root,
     stdout=b"<>\n<a b>\n<$(touch /tmp/should-not-exist);*>\n<--help>\n<-->\n")
blob = bytes(range(256)) * 4
r = call(["--", "sh", "-c", "cat; printf separate-stderr >&2"], root, data=blob, stdout=blob, err=b"separate-stderr")
assert r.stderr == b"separate-stderr"
emit("binary I/O: all 1024 bytes equal; stderr separate")
for cmd in ["sh", "/bin/sh"]:
    expected = f"{root}/nested\n{cmd}\n".encode()
    call(["--", cmd, "-c", 'pwd; printf "%s\\n" "$HERDR_AGENT"'], root / "nested",
         extra={"PINFOLD_ENV_HERDR_AGENT": "conflicting-label"}, stdout=expected)
call(["--", "sh", "-c", 'printf home-edited > "$HOME/keep"; printf project-edited > keep'], root)
call(["--", "sh", "-c", 'cat "$HOME/keep"; printf "\\n"; cat keep; printf "\\n"; test ! -t 0; test ! -t 1'],
     root, stdout=b"home-edited\nproject-edited\n")
# The command itself acknowledges readiness; no sleep or readiness retry.
argv = [binary, "--", "sh", "-c", 'echo FIRST-READY; read line; printf "FIRST-%s\\n" "$line"']
first = sp.Popen(argv, cwd=root, env=env, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
assert first.stdout.readline() == b"FIRST-READY\n"
emit('$ $BIN -- sh -c \'echo FIRST-READY; read line; printf "FIRST-%s\\n" "$line"\' (held after FIRST-READY)')
call(["--", "printf", "SECOND\\n"], root, expect=125, stdout=b"", err=b"disk-in-use")
out, errors = first.communicate(b"still-executes\n", timeout=120)
assert first.returncode == 0 and out == b"FIRST-still-executes\n" and not errors
emit("first exit=0 stdout=FIRST-still-executes; stderr empty")
call(["--", "printf", "SECOND\\n"], root, stdout=b"SECOND\n")
missing = project("probe-missing")
missing_cfg = config(missing)
missing_cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = "localhost/pinfold/image-probe-absent:latest"\n')
call(["--", "true"], missing, expect=125, err=b"image-missing")
no_state(missing)
missing_cfg.write_text(cfg.read_text())
call(["--", "true"], missing)
protected = project("probe-protected")
config(protected)
cache = protected / "inside\nconfig: FORGED"
extra = {"XDG_CACHE_HOME": str(cache)}
call(["--", "true"], protected, expect=125, extra=extra, err=b"host-path-in-project")
no_state(protected)
assert not cache.exists()
doctor = call(["doctor"], protected, extra=extra)
assert b"\nconfig: FORGED" not in doctor.stdout and b"\\nconfig: FORGED" in doctor.stdout
call(["--", "true"], protected)
newline = project("probe-newline\nconfig: FORGED")
r = call(["doctor"], newline)
assert b"\nconfig: FORGED" not in r.stdout and b"\\nconfig: FORGED" in r.stdout
call(["--", "true"], newline, expect=125, err=b"no-config")
normal = work / "probe-corrected"
newline.rename(normal)
config(normal)
call(["--", "true"], normal)
plain = project("probe-folder", git=False)
(plain / "host-only").write_text("never-import\n")
if linux:
    call(["--", "true"], plain, expect=125, err=b"not-git")
else:
    # Non-Git config creation itself must reject roots inside this project.
    inside = {"XDG_CONFIG_HOME": str(plain / "internal")}
    call(["config", "edit"], plain, expect=1, extra=inside, err=b"host-path-in-project")
    assert not (plain / "internal").exists()
    config(plain)
    call(["--", "sh", "-c", 'test ! -e host-only; test "$PWD" = "$1"; printf folder > keep; printf folder-home > "$HOME/keep"', "sh", str(plain)], plain)
    call(["--", "sh", "-c", 'test ! -e host-only; cat keep; printf "\\n"; cat "$HOME/keep"; printf "\\n"'],
         plain, stdout=b"folder\nfolder-home\n")
    report = call(["config"], plain)
    disks = json.loads(report.stdout)["project"]["disks"]
    assert disks["home"] and disks["project"]
    emit("non-Git report names both persistent disks")
# Retained terminal records contain no command argv or output.
ends = list((work / "s/pinfold/runs").glob("*/terminal.json"))
if not ends:
    ends = list((work / "s/pinfold/runs").glob("*/terminal.jsonl"))
assert ends and all(json.loads(path.read_text())["reason"] == "sessions-ended" for path in ends)
assert not any(b"FIRST-READY" in path.read_bytes() or b'"argv"' in path.read_bytes() for path in (work / "s/pinfold/runs").glob("*/*.json"))
emit("terminal records=" + str(len(ends)) + "; all sessions-ended; no argv or fixture output")
emit("PASS common cases; work=" + str(work))
