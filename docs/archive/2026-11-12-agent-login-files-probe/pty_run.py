"""Run argv[2:] on a new terminal, copy its output to the file argv[1],
and send it no input. Exits with the command's status."""

import os
import pty
import sys

pid, master = pty.fork()
if pid == 0:
    os.execvp(sys.argv[2], sys.argv[2:])
with open(sys.argv[1], "wb") as out:
    while True:
        try:
            data = os.read(master, 4096)
        except OSError:
            break
        if not data:
            break
        out.write(data)
        out.flush()
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
