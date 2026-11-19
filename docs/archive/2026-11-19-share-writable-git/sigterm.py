#!/usr/bin/env python3
"""Review fix 1: SIGTERM while the git-optional-locks warning is blocked.

Usage: sigterm.py BIN PROJECT. PROJECT is a share project whose box is kept
by `pinfold start`, so a join writes nothing to stderr before the warning.
First three joins with a read stderr time the warning. Then three joins get
a stderr pipe filled to capacity and never read; SIGTERM goes 0.5 s after
the measured warning time, inside the write's one-second deadline. CMD
creates a marker file named for the trial.
"""
import fcntl, os, signal, subprocess, sys, time

BIN, PROJECT = sys.argv[1], sys.argv[2]
ENV = {k: v for k, v in os.environ.items() if k != "GIT_OPTIONAL_LOCKS"}

times = []
for i in range(3):
    start = time.monotonic()
    p = subprocess.Popen([BIN, "--", "true"], cwd=PROJECT, env=ENV, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    line = p.stderr.readline()
    times.append(time.monotonic() - start)
    p.stderr.read()
    print(f"timing {i}: warning after {times[-1]:.2f} s: {line.decode().strip()[:40]}..., exit {p.wait()}")
at = max(times) + 0.5

for i in range(3):
    marker = os.path.join(PROJECT, f"ran-{i}")
    r, w = os.pipe()
    fcntl.fcntl(w, fcntl.F_SETFL, os.O_NONBLOCK)
    filled = 0
    try:
        while True:
            filled += os.write(w, b"x" * 512)
    except BlockingIOError:
        pass
    fcntl.fcntl(w, fcntl.F_SETFL, 0)
    p = subprocess.Popen([BIN, "--", "touch", marker], cwd=PROJECT, env=ENV,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=w)
    os.close(w)
    time.sleep(at)
    p.send_signal(signal.SIGTERM)
    rc = p.wait()
    time.sleep(2)
    print(f"trial {i}: pipe held {filled} bytes, SIGTERM at {at:.2f} s, exit {rc}, "
          f"CMD ran: {'yes' if os.path.exists(marker) else 'no'}")
    os.close(r)
