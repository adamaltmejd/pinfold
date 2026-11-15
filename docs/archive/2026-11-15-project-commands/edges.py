#!/usr/bin/env python3
"""Additional CLI controls and record checks. Usage: edges.py BINARY common-root."""
import hashlib
import http.server
import json
import os
from pathlib import Path
import shlex
import subprocess as sp
import sys
import threading

binary = str(Path(sys.argv[1]).resolve())
work = Path(sys.argv[2])
root = work / "probe-commands"
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G")

def scrub(text):
    return text.encode("utf-8", "backslashreplace").decode().replace(binary, "$BIN").replace(str(work), "$W").replace(str(Path.home()), "/home/user" if sys.platform.startswith("linux") else "/Users/user")

def run(args, code=0, cwd=root, extra=None, output=None):
    result = sp.run([binary] + args, cwd=cwd, env=dict(env, **(extra or {})), capture_output=True, timeout=120)
    print(scrub("$ " + shlex.join(["$BIN"] + [os.fsdecode(a) for a in args])), flush=True)
    print(scrub(f"exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}"), flush=True)
    assert result.returncode == code
    if output is not None:
        assert result.stdout == output
    return result

for args in [[""], ["pi"], ["attach"]]:
    r = run(args, 1)
    assert b"pinfold -- CMD" in r.stderr
run(["--", "printf", b"\xff"], 125)
run(["box", b"\xff"], 1)
run(["--", "printf", "corrected\\n"], output=b"corrected\n")
run(["--", "probe-command-does-not-exist"], 127)
run(["--", "/"], 126)
run(["--", "true"])
link = work / "canonical-link"
if not link.is_symlink():
    link.symlink_to(root / "nested")
run(["--", "/bin/sh", "-c", 'pwd; printf "%s\\n" "$HERDR_AGENT"'], cwd=link,
    extra={"PINFOLD_ENV_HERDR_AGENT": "conflict"}, output=f"{root}/nested\n/bin/sh\n".encode())
shim = work / "pi"
if not shim.is_symlink():
    shim.symlink_to(binary)
r = sp.run([str(shim), "--version"], cwd=root, env=env, capture_output=True)
assert r.returncode == 0 and r.stdout.startswith(b"pinfold ")
print(f"$ $W/pi --version; exit=0 stdout={r.stdout!r}; symlink uses ordinary CLI", flush=True)
# A newline in the selected profile lookup path cannot forge a doctor finding.
other = work / "profile\nconfig: FORGED"
ident = root.name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg = other / "pinfold/projects" / ident / "pinfold.toml"
cfg.parent.mkdir(parents=True, exist_ok=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nprofile = "probe-missing"\n')
r = run(["doctor"], extra={"XDG_CONFIG_HOME": str(other)})
assert b"\nconfig: FORGED" not in r.stdout and b"\\nconfig: FORGED" in r.stdout
run(["--", "true"])
# Fixture-only injecting route. The value is supplied in env, never argv or files.
seen = []
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        seen.append(self.headers.get("X-Probe"))
        self.send_response(200)
        self.send_header("Content-Length", "8")
        self.end_headers()
        self.wfile.write(b"route-ok")
    def log_message(self, *_):
        pass
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
script = 'exec 3<>/dev/tcp/127.0.0.1/3128; printf "GET http://probe.internal/check HTTP/1.1\\r\\nHost: probe.internal\\r\\nConnection: close\\r\\n\\r\\n" >&3; cat <&3'
main_cfg = work / "c/pinfold/projects" / ident / "pinfold.toml"
original = main_cfg.read_text()
main_cfg.write_text(original + f'\n[routes."probe.internal"]\nto = "http://127.0.0.1:{server.server_port}"\n[routes."probe.internal".headers."X-Probe"]\nfrom = "PINFOLD_PROBE_TOKEN"\n')
try:
    run(["--", "bash", "-c", script], extra={"PINFOLD_PROBE_TOKEN": "fixture-only-value"})
finally:
    main_cfg.write_text(original)
server.shutdown()
assert seen == ["fixture-only-value"]
records = list((work / "s/pinfold/runs").glob("*/*.json"))
assert not any(b"fixture-only-value" in path.read_bytes() or b'"argv"' in path.read_bytes() or b"FIRST-READY" in path.read_bytes() for path in records)
ends = list((work / "s/pinfold/runs").glob("*/terminal.json"))
reasons = [json.loads(path.read_text())["reason"] for path in ends]
assert ends and set(reasons) <= {"sessions-ended", "signal"} and "sessions-ended" in reasons
print(f"records: sessions-ended={reasons.count('sessions-ended')} signal={reasons.count('signal')}; no argv, command output or fixture credential", flush=True)
print("PASS CLI controls, canonical symlink cwd, escaping, synthetic route, records", flush=True)
