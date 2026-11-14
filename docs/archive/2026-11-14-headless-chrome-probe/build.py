#!/usr/bin/env python3
"""Build only probe-owned profiles; keep all runtime state under /tmp."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2].parent
OUT = Path(__file__).resolve().parent
BIN = ROOT / "target/debug/pinfold"
W = Path(tempfile.mkdtemp(prefix="pfc.", dir="/tmp")).resolve()
(OUT / "scratch-path").write_text(str(W))
ENV = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
ENV.update(XDG_CONFIG_HOME=str(W / "config"), XDG_STATE_HOME=str(W / "state"), PINFOLD_NO_UPDATE_CHECK="1")
ENV.pop("XDG_CACHE_HOME", None)
(W / "state/pinfold").mkdir(parents=True)
(W / "state/pinfold/maintenance").touch()

def scrub(value):
    return value.replace(str(Path.home()), "/Users/user").replace(str(W), "$W").replace(str(W).removeprefix("/private"), "$W")

def run(args, name):
    start = time.monotonic()
    result = subprocess.run(list(map(str, args)), env=ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    elapsed = time.monotonic() - start
    output = f"$ {scrub(' '.join(map(str, args)))}\n{scrub(result.stdout)}\nexit={result.returncode}; elapsed_seconds={elapsed:.3f}\n"
    (OUT / name).write_text(output)
    print(output, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)

run(["container", "system", "status"], "runtime.txt")
run([BIN, "profile", "new", "probe-chrome-base"], "profile-new.txt")
profiles = W / "config/pinfold/profiles"
for name, base, packages in [
    ("probe-chrome", "probe-chrome-base", "chromium"),
    ("probe-chrome-tools", "probe-chrome", "strace python3"),
]:
    p = profiles / name
    p.mkdir()
    (p / "pinfold.toml").write_text("")
    recipe = f"""FROM localhost/pinfold/profile-{base}:latest
RUN mkdir -p /opt/{name}; \\
    (set -eux; export DEBIAN_FRONTEND=noninteractive; \\
    apt-get -o Debug::Acquire::http=true update; \\
    apt-get -o Debug::Acquire::http=true install -y --no-install-recommends {packages}; \\
    rm -rf /var/lib/apt/lists/*; \\
    find / -xdev -perm /6000 -type f -exec chmod a-s {{}} +) > /opt/{name}/build.log 2>&1
"""
    (p / "Containerfile").write_text(recipe)
    (OUT / f"{name}.Containerfile").write_text(recipe)
(OUT / "probe-chrome-base.Containerfile").write_bytes((profiles / "probe-chrome-base/Containerfile").read_bytes())
for name in ["probe-chrome-base", "probe-chrome", "probe-chrome-tools"]:
    run(["df", "-g", "/"], f"{name}-disk.txt")
    available = int(subprocess.check_output(["df", "-g", "/"], text=True).splitlines()[1].split()[3])
    if available < 12:
        raise SystemExit("under 12 GiB: no build")
    run([BIN, "build", "--profile", name], f"{name}-build.txt")
    run(["container", "image", "inspect", f"localhost/pinfold/profile-{name}:latest"], f"{name}-image.json.txt")
