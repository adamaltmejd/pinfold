#!/usr/bin/env python3
"""Run on Linux: python3 linux.py /absolute/path/to/pinfold."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve(strict=True))
tracer = shutil.which("strace")
with tempfile.TemporaryDirectory(prefix="pfp-linux-", dir="/tmp") as temporary:
    scratch = Path(temporary)
    fixture = scratch / "share"
    fixture.mkdir()
    (fixture / "ordinary").write_text("fixture\n")
    home = scratch / "home"
    home.mkdir()
    print("Linux", os.uname().release, "uid", os.getuid())
    print("strace", bool(tracer), "runtime PATH empty; isolated cache")
    base = {"name": "probe-order", "image": "probe-absent-ordering"}
    mount = {"host": str(fixture), "guest": "/work"}
    cases = [
        ("malformed", "{"),
        ("unknown-kind", json.dumps(dict(base, mounts=[dict(mount, kind="share")]))),
        ("invalid-parsed", json.dumps(dict(base, mounts=[dict(mount, guest="relative")]))),
        ("readonly-control", json.dumps(dict(base, mounts=[mount]))),
    ]
    def clean(value):
        return str(value).replace(temporary, "/tmp/pfp-linux").replace(binary, "PINFOLD")

    def inventory(root):
        return sorted(str(p.relative_to(root)) for p in root.rglob("*")) if root.exists() else []

    for label, spec in cases:
        root = scratch / label
        root.mkdir()
        roots = {key: root / key for key in ("config", "state", "cache")}
        environment = {
            "HOME": str(home), "PATH": "/nonexistent", "LANG": "C",
            "PINFOLD_NO_UPDATE_CHECK": "1",
            **{"XDG_" + key.upper() + "_HOME": str(path) for key, path in roots.items()},
        }
        print(label, "before", {key: path.exists() for key, path in roots.items()})
        command = [binary, "box", "up"]
        trace = root / "trace"
        if tracer:
            command = [tracer, "-f", "-qq", "-s", "180", "-e", "trace=%file,read",
                       "-o", str(trace), *command]
        result = subprocess.run(command, input=spec + "\n", text=True, capture_output=True,
                                env=environment, cwd=scratch, timeout=45)
        print(label, "exit", result.returncode)
        for line in (result.stdout + result.stderr).splitlines():
            print(clean(line))
        print(label, "after", {key: inventory(path) for key, path in roots.items()})
        if trace.exists():
            selected = [line for line in trace.read_text().splitlines()
                        if "maintenance" in line or 'readlink("/proc/self/exe"' in line
                        or "read(0," in line]
            for line in selected:
                print("TRACE", clean(line))
    print("readonly-control uses an absent image; successful runtime startup not established")
print("scratch removed")
