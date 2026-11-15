#!/usr/bin/env python3
"""Retain diagnostic excerpts, then remove only these fixtures' disks."""
import hashlib
import json
import os
from pathlib import Path
import subprocess as sp
import sys

binary = str(Path(sys.argv[1]).resolve())
work = Path(sys.argv[2])
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G", EDITOR="true", VISUAL="true")

def run(args, root=work, extra=None):
    result = sp.run([binary] + args, cwd=root, env=dict(env, **(extra or {})), capture_output=True, timeout=120)
    return result

def emit(text):
    print(text.replace(str(work), "$W").replace(str(Path.home()), "/home/user" if sys.platform.startswith("linux") else "/Users/user"), flush=True)

newline = work / "probe-newline\nconfig: FORGED"
newline.mkdir(exist_ok=True)
sp.run(["git", "init", "-q", str(newline)], check=True)
for root, extra in [(newline, {}), (work / "probe-protected", {"XDG_CACHE_HOME": str(work / "probe-protected/inside\nconfig: FORGED")})]:
    result = run(["doctor"], root, extra)
    assert result.returncode == 0 and b"\nconfig: FORGED" not in result.stdout
    emit(f"$ $BIN doctor; cwd={str(root)!r}; overrides={extra!r}")
    for line in result.stdout.decode().splitlines():
        if line.startswith(("pinfold doctor:", "config:")):
            emit(line)
if sys.platform == "darwin":
    folder = work / "probe-folder"
    result = run(["config"], folder)
    assert result.returncode == 0
    emit("$ $BIN config; cwd=$W/probe-folder\nproject=" + json.dumps(json.loads(result.stdout)["project"]))
    guarded = work / "probe-folder-guard"
    guarded.mkdir(exist_ok=True)
    assert run(["config", "edit"], guarded).returncode == 0
    ident = guarded.name + "-" + hashlib.sha256(os.fsencode(guarded)).hexdigest()[:12]
    cfg = work / "c/pinfold/projects" / ident / "pinfold.toml"
    image = json.loads(result.stdout)["image"]
    cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = ' + json.dumps(image) + '\n')
    result = run(["--", "true"], guarded, {"XDG_CACHE_HOME": str(guarded / "inside")})
    assert result.returncode == 125 and b"host-path-in-project" in result.stderr
    assert not (work / "s/pinfold/projects" / ident).exists()
    emit(f"$ XDG_CACHE_HOME=$W/probe-folder-guard/inside $BIN -- true; exit={result.returncode}; stderr={result.stderr!r}; project state absent")
    assert run(["--", "true"], guarded).returncode == 0
    emit("$ $BIN -- true; same non-Git folder with external roots; exit=0")
records = [p for p in (work / "s/pinfold/runs").glob("*/*") if p.suffix in {".json", ".jsonl"}]
assert not any(b"fixture-only-value" in p.read_bytes() or b'"argv"' in p.read_bytes() or b"FIRST-READY" in p.read_bytes() for p in records)
emit(f"retained JSON/JSONL files scanned={len(records)}; no fixture credential, argv field, or command-output marker")
projects = {p.name for p in (work / "c/pinfold/projects").iterdir()}
boxes = run(["box", "list", "--label", "dev.pinfold.project"])
assert boxes.returncode == 0
owned = [json.loads(line) for line in boxes.stdout.splitlines() if json.loads(line)["labels"]["dev.pinfold.project"] in projects]
assert not owned
emit("$ $BIN box list --label dev.pinfold.project; own fixture boxes=0")
removed = run(["disk", "rm", "--all", "--json"])
assert removed.returncode == 0
lines = [json.loads(line) for line in removed.stdout.splitlines()]
assert all(line["event"] == "removed" for line in lines)
emit(f"$ $BIN disk rm --all --json; exit=0; removed={len(lines)} (isolated state root)")
after = run(["disk", "list", "--json"])
assert after.returncode == 0 and not after.stdout
emit("$ $BIN disk list --json; exit=0; stdout empty")
