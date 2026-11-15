#!/usr/bin/env python3
"""Real PTY review cases: tty-review.py BIN IMAGE baseline|fixed [fresh-root]."""
import ctypes
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import select
import signal
import struct
import subprocess as sp
import sys
import tempfile
import termios
import time

binary = str(Path(sys.argv[1]).resolve())
image, phase = sys.argv[2:4]
fixed = phase == "fixed"
work = Path(sys.argv[4] if len(sys.argv) > 4 else tempfile.mkdtemp(prefix="pfreviewtty.", dir="/tmp")).resolve()
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G", TERM="xterm-256color")
(work / "s/pinfold").mkdir(parents=True)
(work / "s/pinfold/maintenance").touch()
root = work / "probe-terminal"
root.mkdir()
sp.run(["git", "init", "-q", str(root)], check=True)
sp.run(["git", "-C", str(root), "-c", "user.name=user", "-c", "user.email=user@example.invalid", "commit", "--allow-empty", "-qm", "fixture"], check=True)
ident = root.name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg = work / "c/pinfold/projects" / ident / "pinfold.toml"
cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = ' + json.dumps(image) + '\n')

def emit(text):
    print(text.replace(binary, "$BIN").replace(str(work), "$W"), flush=True)

def deny_raw_setup():
    # The real kernel refuses only this child's terminal-mode-setting ioctls.
    class Filter(ctypes.Structure):
        _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte), ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint)]
    class Program(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ushort), ("filter", ctypes.POINTER(Filter))]
    rules = (Filter * 8)(Filter(0x20, 0, 0, 0), Filter(0x15, 0, 4, 16), Filter(0x20, 0, 0, 24),
                          Filter(0x15, 3, 0, 0x5402), Filter(0x15, 2, 0, 0x5403), Filter(0x15, 1, 0, 0x5404),
                          Filter(0x06, 0, 0, 0x7fff0000), Filter(0x06, 0, 0, 0x00050001))
    program = Program(len(rules), rules)
    libc = ctypes.CDLL(None, use_errno=True)
    assert libc.prctl(38, 1, 0, 0, 0) == 0
    assert libc.prctl(22, 2, ctypes.byref(program), 0, 0) == 0

def configured(attributes):
    attributes = list(attributes)
    if sys.platform == "darwin":
        # Darwin marks pending input with PENDIN; it is terminal state.
        attributes[3] &= ~0x20000000
    return attributes

def terminal(args, interrupt=False, redirected=False, size=(36, 120), deny_raw=False, flood_interrupt=False):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", *size, 0, 0))
    pid = os.fork()
    if pid == 0:
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for fd in range(3):
            os.dup2(slave, fd)
        os.tcsetpgrp(0, os.getpgrp())
        os.close(master)
        os.close(slave)
        os.chdir(root)
        before = termios.tcgetattr(0)
        before_mode = sp.run(["stty", "-g"], capture_output=True, check=True).stdout.strip()
        if deny_raw:
            deny_raw_setup()
        target = open(work / "redirected", "wb") if redirected else None
        child = sp.Popen([binary] + args, env=env, stdout=target)
        os.write(1, f"DRIVER_PID={child.pid}\n".encode())
        code = child.wait()
        if target:
            target.close()
        after = termios.tcgetattr(0)
        after_mode = sp.run(["stty", "-g"], capture_output=True, check=True).stdout.strip()
        os.write(1, f"DRIVER_STTY before={before_mode!r} after={after_mode!r}\n".encode())
        sane = bool(after[3] & termios.ICANON and after[3] & termios.ECHO and after[3] & termios.ISIG)
        os.write(1, f"DRIVER_RESULT={code},{configured(before) == configured(after)},{sane}\n".encode())
        os._exit(0)
    output, sent = b"", False
    deadline = time.monotonic() + 60
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.2)[0]:
                chunk = os.read(master, 65536)
                output += chunk
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).replace(b"\r", b"")
            if interrupt and b"CMD-READY\n" in plain and not sent:
                cmd_pid = int(re.search(rb"DRIVER_PID=([0-9]+)", plain)[1])
                if flood_interrupt:
                    # Leave the output queue full, so exec cannot drop its raw guard.
                    while time.monotonic() < deadline:
                        queued = struct.unpack("I", fcntl.ioctl(master, termios.FIONREAD, bytes(4)))[0]
                        if queued >= 4095:
                            break
                    else:
                        raise RuntimeError("PTY output did not fill")
                    pidfd = os.pidfd_open(cmd_pid)
                    os.kill(cmd_pid, signal.SIGTERM)
                    if not select.select([pidfd], [], [], max(0, deadline - time.monotonic()))[0]:
                        os.close(pidfd)
                        raise RuntimeError("pinfold did not exit while output remained blocked")
                    os.close(pidfd)
                    stat = Path(f"/proc/{cmd_pid}/stat")
                    try:
                        state = stat.read_text().rsplit(")", 1)[1].split()[0]
                    except FileNotFoundError:
                        state = "gone"
                    assert state in {"Z", "gone"}
                    emit(f"blocked-output queued={queued}; SIGTERM sent; pinfold exited before draining")
                else:
                    os.kill(cmd_pid, signal.SIGTERM)
                sent = True
            if os.waitpid(pid, os.WNOHANG)[0]:
                while select.select([master], [], [], 0)[0]:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        break
                    output += chunk
                break
        else:
            os.killpg(pid, signal.SIGTERM)
            os.waitpid(pid, 0)
            raise RuntimeError("terminal case deadline")
        plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).replace(b"\r", b"")
        match = re.search(rb"DRIVER_RESULT=(-?[0-9]+),(True|False),(True|False)", plain)
        assert match, plain
        code, equal, sane = int(match[1]), match[2] == b"True", match[3] == b"True"
        emit(f"$ $BIN {args!r}; host-size={size}; redirected={redirected}; deny-TCSETS={deny_raw}; blocked-output={flood_interrupt}")
        observed = plain.replace(b"\0", b"")
        emit(f"exit={code} termios-restored={equal} stty-sane={sane} zero-bytes={plain.count(bytes([0]))} output={observed!r}")
        if redirected:
            emit("redirected bytes=" + repr((work / "redirected").read_bytes()))
        return code, equal, sane, plain
    finally:
        os.close(master)
        os.close(slave)

emit("phase=" + phase + " work=" + str(work))
code, equal, sane, output = terminal(["--", "sh", "-c", "echo CMD-READY; read line"], interrupt=True)
assert code == 143
if fixed:
    assert equal and sane
if sys.platform.startswith("linux"):
    code, equal, sane, output = terminal(["--", "sh", "-c", "echo CMD-READY; head -c 1048576 /dev/zero; read line"], interrupt=True, flood_interrupt=True)
    assert code == 143
    if fixed:
        assert equal and sane
# Public box exec --tty must also work with a non-terminal stdout.
name = "probe-review-tty-" + str(os.getpid())
owner = sp.Popen([binary, "box", "up"], cwd=root, env=env, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE)
metadata = "36,120:-:-:-:-:-" if fixed else "36,120:-:-:-"
owner.stdin.write(json.dumps({"name": name, "image": image, "cpus": 2, "memory": "1G", "env": {"PINFOLD_EXEC_METADATA": metadata}}).encode() + b"\n")
owner.stdin.flush()
ready = json.loads(owner.stdout.readline())
assert ready["event"] == "ready", ready
try:
    # Real trusted init sees requested dimensions on a pipe and its ioctl fails.
    cmd = ["box", "exec", name, "--", "/proc/self/exe", "init", "exec", "--", "printf", "IOCTL-MUST-NOT-START"]
    ioctl_failure = sp.run([binary] + cmd, cwd=root, env=env, capture_output=True, timeout=60)
    emit(f"$ $BIN {cmd!r}; restored metadata={metadata!r}")
    emit(f"exit={ioctl_failure.returncode} stdout={ioctl_failure.stdout!r} stderr={ioctl_failure.stderr!r}")
    assert ioctl_failure.returncode == 125 and b"terminal size" in ioctl_failure.stderr and b"IOCTL-MUST-NOT-START" not in ioctl_failure.stdout
    code, _, _, _ = terminal(["box", "exec", name, "--tty", "--", "printf", "REDIRECTED"], redirected=True)
    assert code == (0 if fixed or sys.platform.startswith("linux") else 1)
    if fixed:
        assert (work / "redirected").read_bytes().replace(b"\r", b"") == b"REDIRECTED"
    code, _, _, output = terminal(["box", "exec", name, "--tty", "--", "stty", "size"], size=(0, 0))
    assert code == 0
    if sys.platform.startswith("linux"):
        code, _, _, output = terminal(["--", "sh", "-c", "echo CMD-MUST-NOT-START"], deny_raw=True)
        if fixed:
            assert code == 125 and b"CMD-MUST-NOT-START" not in output
        code, _, _, output = terminal(["box", "exec", name, "--tty", "--", "sh", "-c", "echo CORE-MUST-NOT-START"], deny_raw=True)
        if fixed:
            assert code == 1 and b"CORE-MUST-NOT-START" not in output
finally:
    sp.run([binary, "box", "down", name], cwd=root, env=env, check=True, capture_output=True)
    owner.communicate(timeout=120)
sp.run([binary, "disk", "rm", "--all", "--json"], cwd=root, env=env, check=True, capture_output=True)
emit("PASS terminal review cases")
