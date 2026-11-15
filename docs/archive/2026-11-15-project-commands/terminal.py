#!/usr/bin/env python3
"""Real host PTY cases. Usage: terminal.py BINARY existing-common-root."""
import fcntl
import os
import re
from pathlib import Path
import select
import signal
import struct
import subprocess
import sys
import termios
import time

binary = str(Path(sys.argv[1]).resolve())
work = Path(sys.argv[2])
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work / "c"), XDG_STATE_HOME=str(work / "s"),
           PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G",
           TERM="xterm-256color", COLORTERM="truecolor",
           PINFOLD_ENV_TERM="wrong-term", PINFOLD_ENV_COLORTERM="wrong-color")

def execute(argv, expected, resize=False):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 0, 0))
    pid = os.fork()
    if pid == 0:
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for fd in range(3):
            os.dup2(slave, fd)
        os.tcsetpgrp(0, os.getpgrp())
        os.close(master)
        os.close(slave)
        os.chdir(work / "probe-commands")
        attributes = termios.tcgetattr(0)
        flags = fcntl.fcntl(0, fcntl.F_GETFL)
        result = subprocess.run([binary, "--"] + argv, env=env)
        after = termios.tcgetattr(0)
        after_flags = fcntl.fcntl(0, fcntl.F_GETFL)
        mutable = os.O_NONBLOCK | os.O_APPEND | os.O_ASYNC | os.O_SYNC | os.O_DSYNC
        restored = after_flags & mutable == flags & mutable and after == attributes
        os.write(1, f"DRIVER_RESTORED={restored} ATTRS_EQUAL={after == attributes} FLAGS={flags},{after_flags}\n".encode())
        os._exit(result.returncode)
    output = b""
    resized = False
    status = None
    deadline = time.monotonic() + 30
    try:
        while time.monotonic() < deadline:
            readable, _, _ = select.select([master], [], [], 0.2)
            if readable:
                output += os.read(master, 65536)
            if resize and b"READY\n" in output.replace(b"\r", b"") and not resized:
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 41, 133, 0, 0))
                os.killpg(pid, signal.SIGWINCH)
                print(f"host resize={struct.unpack('HHHH', fcntl.ioctl(master, termios.TIOCGWINSZ, bytes(8)))}; SIGWINCH sent to PTY foreground group", flush=True)
                resized = True
            found, status = os.waitpid(pid, os.WNOHANG)
            if found:
                while select.select([master], [], [], 0)[0]:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        break
                    output += chunk
                break
        else:
            print(f"terminal deadline resized={resized} raw={output!r}", flush=True)
            os.killpg(pid, signal.SIGTERM)
            os.waitpid(pid, 0)
            raise RuntimeError("command deadline")
        raw_output = output
        restored = b"DRIVER_RESTORED=True" in output
        output = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).replace(b"\r", b"")
        output = re.sub(rb"DRIVER_RESTORED=True ATTRS_EQUAL=(True|False) FLAGS=[0-9]+,[0-9]+\n", b"", output)
        print(f"$ $BIN -- {argv!r}; host=36x120 TERM=xterm-256color COLORTERM=truecolor", flush=True)
        print(f"exit={os.waitstatus_to_exitcode(status)} raw={raw_output!r} output={output!r} termios-and-mutable-stdin-flags-restored={restored}", flush=True)
        assert os.waitstatus_to_exitcode(status) == 0 and output == expected.replace(b"\r", b"") and restored
    finally:
        os.close(master)
        os.close(slave)

# Each command starts in a distinct new box. Inspect immediately, never wait for correction.
execute(["stty", "size"], b"36 120\r\n")
execute(["sh", "-c", 'printf "%s %s\\n" "$TERM" "$COLORTERM"; stty size'],
        b"xterm-256color truecolor\r\n36 120\r\n")
# Both guests are Linux. 0x5413 is their TIOCGWINSZ.
# Perl observes the native resize signal without shell trap/read behavior.
execute(["perl", "-MPOSIX", "-e",
         '$|=1; $SIG{WINCH}=sub{my $s=pack("S4",0,0,0,0); ioctl(STDIN,0x5413,$s) or die $!; my($r,$c)=unpack("S4",$s); print "RESIZED $r $c\\n"; exit}; print "READY\\n"; POSIX::pause();'],
        b"READY\r\nRESIZED 41 133\r\n", resize=True)
print("PASS immediate dimensions, terminal identity, resize, termios and mutable-flag restoration", flush=True)
