#!/usr/bin/env python3
"""Extend the retained PR 1 PTY driver; no binary hooks or suite tests."""
from pathlib import Path
import sys

base = Path(__file__).resolve().parent.parent
source = (base / '2026-11-15-project-command-review/tty-review.py').read_text()
source = source[:source.index('emit("phase=" + phase')]
source = source.replace('flood_interrupt=False):', 'flood_interrupt=False, already_raw=False):')
source = source.replace('        before = termios.tcgetattr(0)',
    '        if already_raw:\n            import tty\n            tty.setraw(0)\n        before = termios.tcgetattr(0)')
source = source.replace('if queued >= 4095:', 'if not select.select([], [slave], [], 0)[1]:')
source = source.replace('                    pidfd = os.pidfd_open(cmd_pid)', '''                    if sys.platform == 'darwin':
                        emit(f'BLOCKED_CLIENT={cmd_pid} queued={queued}')
                        queue = select.kqueue()
                        queue.control([select.kevent(cmd_pid, filter=select.KQ_FILTER_PROC,
                            flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT, fflags=select.KQ_NOTE_EXIT)], 0, 0)
                        os.kill(cmd_pid, signal.SIGTERM)
                        events = queue.control(None, 1, 5 if case_mode=='diagnose' else max(0, deadline-time.monotonic()))
                        if not events and case_mode=='diagnose':
                            processes = sp.check_output(['ps','-axo','pid,ppid,stat,wchan,comm']).decode()
                            children = [x for x in processes.splitlines() if len(x.split())>1 and x.split()[1]==str(cmd_pid)]
                            emit('BLOCKED_CHILDREN=' + repr(children))
                            for childline in children:
                                sp.run(['sample',childline.split()[0],'1','10','-file',str(work/'native.sample')],stdout=sp.DEVNULL,stderr=sp.DEVNULL)
                        assert events
                        queue.close()
                        emit(f"blocked-output queued={queued}; pinfold exited before draining")
                        sent = True
                        continue
                    pidfd = os.pidfd_open(cmd_pid)''')
# sandbox_init restricts this real Darwin child and descendants, never the host.
source = source.replace('    # The real kernel refuses only', '''    if sys.platform == 'darwin':
        library = ctypes.CDLL('/usr/lib/libsandbox.1.dylib')
        error = ctypes.c_char_p()
        profile = ('(version 1)(allow default)(deny file-ioctl (ioctl-command %d) '
                   '(ioctl-command %d) (ioctl-command %d))' %
                   (0x80487414, 0x80487415, 0x80487416)).encode()
        assert library.sandbox_init(profile, 0, ctypes.byref(error)) == 0, error.value
        return
    # The real kernel refuses only''')
args = sys.argv[:]
case_mode = args[3] if len(args)>3 else 'all'
sys.argv = [args[0], args[1], args[2], 'fixed']
exec(compile(source, str(base/'2026-11-15-project-command-review/tty-review.py'), 'exec'))
emit('extra-terminal platform=' + sys.platform)
if len(args) < 4 or args[3] != 'remaining':
    code, equal, sane, output = terminal(['--', 'sh', '-c', 'echo CMD-READY; head -c 1048576 /dev/zero; read line'], interrupt=True, flood_interrupt=True)
    assert code == 143 and equal and sane
code, equal, _, output = terminal(['--', 'printf', 'ALREADY-RAW'], already_raw=True)
assert code == 0 and equal and b'ALREADY-RAW' in output
code, _, _, output = terminal(['--', 'sh', '-c', 'echo CMD-MUST-NOT-START'], deny_raw=True)
assert code == 125 and b'CMD-MUST-NOT-START\n' not in output
code, _, _, output = terminal(['--', 'printf', 'SETUP-CONTROL'])
assert code == 0 and b'SETUP-CONTROL' in output
sp.run([binary, 'disk', 'rm', '--all', '--json'], cwd=root, env=env, check=True, capture_output=True)
emit('PASS selected extra terminal cases; blocked-output=' + str(len(args)<4 or args[3]!='remaining'))
terminal_source = (base/'2026-11-15-project-commands/terminal.py').read_text()
terminal_source = terminal_source.replace('work / \"probe-commands\"', 'root').replace('after == attributes', 'configured(after) == configured(attributes)')
sys.argv = [args[0], binary, str(work)]
exec(compile(terminal_source, str(base/'2026-11-15-project-commands/terminal.py'), 'exec'))
sp.run([binary, 'disk', 'rm', '--all', '--json'], cwd=root, env=env, check=True, capture_output=True)
