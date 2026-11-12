"""Run argv[2:] on a new terminal and copy its output to the file argv[1].
Sends the keystrokes in KEYS (a JSON list of [delay_seconds, text]) one by
one, then SIGTERM after a last pause. Sends no credential: only Enter,
Escape, Ctrl-C and slash commands."""

import json
import os
import pty
import select
import signal
import sys
import time

keys = json.loads(os.environ["KEYS"])
pid, master = pty.fork()
if pid == 0:
    os.execvp(sys.argv[2], sys.argv[2:])
with open(sys.argv[1], "wb") as out:
    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([master], [], [], 0.2)
            if ready:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    return False
                if not data:
                    return False
                out.write(data)
                out.flush()
        return True

    for delay, text in keys:
        if not pump(delay):
            break
        os.write(master, text.encode())
    pump(5)
    os.kill(pid, signal.SIGTERM)
    pump(3)
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
