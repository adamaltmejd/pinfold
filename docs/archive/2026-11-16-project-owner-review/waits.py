#!/usr/bin/env python3
"""Manual startup waits: waits.py BIN IMAGE; real hooks and Linux debugger barriers."""
import http.server, json, os, pathlib, select, shlex, signal, subprocess as sp, sys, threading, time
old = pathlib.Path(__file__).with_name('crossing.py')
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0], str(old), 'exec'), globals())
P.mkdir(); git(P, 'init', '-q', '-b', 'review'); (P/'tracked').write_text('FIRST\n'); git(P, 'add', 'tracked')
git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'fixture'); run(['config', 'edit'])
C = W/'c/pinfold/projects'/ident(P)/'pinfold.toml'; C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')

def line(stream, timeout=90):
    assert select.select([stream], [], [], timeout)[0], 'required output barrier absent'
    value = stream.readline(); assert value, 'process ended before barrier'; return value

def busy_launcher():
    receipt, release = threading.Event(), threading.Event()
    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            assert self.path == '/hold'; receipt.set(); assert release.wait(90), 'fixture release absent'
            try: self.send_response(200); self.end_headers(); self.wfile.write(b'SYNC-RELEASED')
            except BrokenPipeError: pass
        def log_message(self, *args): pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture); thread = threading.Thread(target=server.serve_forever); thread.start()
    try:
        C.write_text(C.read_text()+f'routes={{sync="127.0.0.1:{server.server_port}"}}\n'); run(['start'])
        hook = '#!/bin/sh\nexec curl -fsS http://sync/hold\n'
        run(['--', 'sh', '-c', 'mkdir -p .git/hooks; printf %s "$1" >.git/hooks/post-merge; chmod +x .git/hooks/post-merge', 'sh', hook]); run(['stop'])
        (P/'changed').write_text('CHANGED\n'); git(P, 'add', 'changed'); git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'changed')
        first = sp.Popen([B, '--', 'sh', '-c', 'echo FIRST; read x; echo FIRST-DONE'], cwd=P, env=E, stdin=sp.PIPE, stdout=sp.PIPE, stderr=sp.PIPE); children.append(first)
        assert receipt.wait(90), 'real initial Git sync did not reach HTTP barrier'; observed = loc(P)
        second = sp.Popen([B, '--', 'printf', 'SECOND\n'], cwd=P, env=E, stdin=sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE); children.append(second)
        notice = line(second.stderr); assert b'waiting for the project box to start' in notice
        began = time.monotonic(); assert not select.select([first.stdout, second.stdout, second.stderr], [], [], 12)[0], 'busy launcher timed out, repeated notice, or admitted early'
        assert time.monotonic()-began >= 12 and first.poll() is None and second.poll() is None; same(P, observed)
        release.set(); assert line(first.stdout) == b'FIRST\n'
        output, errors = second.communicate(timeout=90); assert second.returncode == 0 and output == b'SECOND\n'
        assert (notice+errors).count(b'waiting for the project box to start') == 1; same(P, observed)
        first.stdin.write(b'release\n'); first.stdin.flush(); first.wait(timeout=180); output, errors = first.communicate(timeout=30)
        assert first.returncode == 0 and output == b'FIRST-DONE\n' and b'owner-timeout' not in errors and not locator(P).exists()
        emit('busy-launch: real HTTP-held initial-sync=12s second notice=1 no-early-CMD heartbeat-kept-starter=True both-clients=0 same-generation=True final-teardown=True')
    finally:
        release.set()
        for child in children:
            if child.poll() is None: child.terminate(); child.communicate(timeout=180)
        run(['stop']); server.shutdown(); server.server_close(); thread.join()

def debugger(mode):
    ready, go, held = [W/(mode+suffix) for suffix in ('.ready', '.go', '.held')]
    for path in (ready, go, held): os.mkfifo(path)
    read = os.open(ready, os.O_RDWR|os.O_NONBLOCK)
    script, output, errors = [W/(mode+suffix) for suffix in ('.gdb', '.out', '.err')]
    gate = 'pinfold::pi::session::owner' if mode == 'silent' else 'pinfold::pi::session::redirect_stderr'
    script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,threading,time
starter=None
def report(value):
 with open({str(ready)!r},'w') as stream:stream.write(json.dumps(value)+'\\n');stream.flush()
def status():
 began=time.monotonic()
 try:
  pid,value=os.waitpid(starter,0);report({{'event':'status','code':os.waitstatus_to_exitcode(value),'seconds':time.monotonic()-began}})
 except Exception as error:report({{'event':'status','unavailable':type(error).__name__}})
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:self.enabled=False;gdb.execute('set follow-fork-mode parent')
  return False
class Gate(gdb.Breakpoint):
 def stop(self):
  owner=gdb.selected_inferior().pid
  if starter is None or owner==starter:return False
  self.enabled=False;value={{'event':'gate','owner':owner,'starter':starter,'stderr':os.readlink('/proc/'+str(owner)+'/fd/2')}}
  report(value)
  if {mode!r}=='silent':status()
  else:threading.Thread(target=status,daemon=True).start()
  with open({str(go)!r},'rb') as stream:assert stream.read(1)==b'G'
  return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Gate({gate!r})
end
run start > {shlex.quote(str(output))} 2> {shlex.quote(str(errors))}
''')
    debug = sp.Popen(['gdb', '-nx', '-q', '-batch', '-x', str(script), '--args', B], cwd=P, env=E, stdout=sp.PIPE, stderr=sp.PIPE)
    pending = b''; observed = None; owner_fd = helper_fd = None; helper = None
    def message(timeout=90):
        nonlocal pending
        deadline = time.monotonic()+timeout
        while b'\n' not in pending:
            assert select.select([read], [], [], max(0, deadline-time.monotonic()))[0], 'required debugger event absent'
            pending += os.read(read, 65536)
        value, pending = pending.split(b'\n', 1); return json.loads(value)
    def release_gate():
        try:
            fd = os.open(go, os.O_WRONLY|os.O_NONBLOCK); os.write(fd, b'G'); os.close(fd)
        except OSError: pass
    try:
        observed = message(); assert observed['event'] == 'gate'; owner_fd = os.pidfd_open(observed['owner'])
        if mode == 'held-writer':
            writer = os.open('/proc/'+str(observed['owner'])+'/fd/2', os.O_WRONLY|os.O_CLOEXEC)
            assert observed['stderr'].startswith('pipe:') and os.fstat(writer).st_ino == os.stat('/proc/'+str(observed['owner'])+'/fd/2').st_ino
            try:
                helper = sp.Popen(['/bin/sh', '-c', 'read x < "$1"', 'sh', str(held)], env=E, stdin=sp.DEVNULL, stdout=sp.DEVNULL, stderr=writer)
            finally: os.close(writer)
            helper_fd = os.pidfd_open(helper.pid)
            assert os.readlink('/proc/'+str(helper.pid)+'/fd/2') == observed['stderr'], 'fixture did not retain original diagnostic pipe'
            release_gate()
        result = message(20); assert result['event'] == 'status'
        if 'unavailable' in result:
            emit(mode+' NOT-RUN: detached starter exit status unavailable'); return
        if mode == 'silent':
            assert result['code'] == 125 and 8 <= result['seconds'] < 16 and b'owner-timeout' in errors.read_bytes()
            release_gate(); debug.communicate(timeout=90); assert debug.returncode == 0
            emit(f'silent-first-reply: required before-heartbeat gate hit; client=125/owner-timeout seconds={result["seconds"]:.2f}; gate released for checked cleanup')
        else:
            assert result['code'] == 0 and not errors.read_bytes() and not select.select([helper_fd], [], [], 0)[0]
            observed_owner = loc(P); assert run(['--', 'printf', 'HELD-WRITER-CONTROL']).stdout == b'HELD-WRITER-CONTROL'; same(P, observed_owner)
            emit('diagnostic-held-writer: independent /bin/sh FIFO fixture retained original fd2 pipe; starter=0 before writer EOF; same-owner command=HELD-WRITER-CONTROL')
            with held.open('wb', buffering=0) as stream: stream.write(b'release\n')
            assert select.select([helper_fd], [], [], 30)[0]; assert helper.wait(timeout=10) == 0; run(['stop']); debug.communicate(timeout=90); assert debug.returncode == 0
        assert run(['--', 'printf', 'RESTART-CONTROL']).stdout == b'RESTART-CONTROL'; run(['stop'])
        emit(mode+' cleanup/restart: marker=RESTART-CONTROL exit=0')
    finally:
        release_gate()
        if helper_fd is not None and not select.select([helper_fd], [], [], 0)[0]: signal.pidfd_send_signal(helper_fd, signal.SIGKILL)
        if helper is not None: helper.wait(timeout=30)
        if owner_fd is not None and debug.poll() is None and not select.select([owner_fd], [], [], 0)[0]: signal.pidfd_send_signal(owner_fd, signal.SIGKILL)
        if debug.poll() is None:
            debug.terminate()
            try: debug.communicate(timeout=15)
            except sp.TimeoutExpired: debug.kill(); debug.communicate(timeout=15)
        for fd in (read, owner_fd, helper_fd):
            if fd is not None: os.close(fd)
        run(['stop'])
try:
    busy_launcher()
    if sys.platform == 'linux': debugger('silent'); debugger('held-writer')
    else: emit('INSPECTED actual descendant inheritance and Mac silent/held-writer windows; Linux independent writer fixture exercises EOF independence')
finally:
    for args in (['stop'], ['disk', 'rm', 'pi-'+ident(P)+'-home'], ['disk', 'rm', 'pi-'+ident(P)+'-project']): cleanup_ok &= invoke(args).returncode == 0
    emit(f'cleanup own-boxes/disks={cleanup_ok} fixture=$W'); assert cleanup_ok
