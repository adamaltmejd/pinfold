#!/usr/bin/env python3
"""Linux debugger fault injection: sync-error.py BIN IMAGE; real held Git sync."""
import http.server, json, os, pathlib, select, shlex, signal, subprocess as sp, sys, threading, time
assert sys.platform == 'linux'
old = pathlib.Path(__file__).with_name('waits.py')
exec(compile(old.read_text().split('def line(')[0], str(old), 'exec'), globals())
R = pathlib.Path(__file__).resolve().parents[3]
SESSION, BOX = R/'crates/pinfold/src/pi/session.rs', R/'crates/pinfold/src/core/box.rs'
def anchor(path, text):
    found = [i+1 for i, value in enumerate(path.read_text().splitlines()) if value.strip() == text]
    assert len(found) == 1, 'source anchor missing/ambiguous: '+text
    return str(path)+':'+str(found[0])
joined_at, down_at = anchor(SESSION, 'let reason = reason?;'), anchor(BOX, 'if self.down {')
receipt, release = threading.Event(), threading.Event()
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        assert self.path == '/hold'; receipt.set(); assert release.wait(150)
        try: self.send_response(200); self.end_headers(); self.wfile.write(b'SYNC-RELEASED')
        except BrokenPipeError: pass
    def log_message(self, *args): pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture); thread = threading.Thread(target=server.serve_forever); thread.start()
ready, go = W/'sync-error.ready', W/'sync-error.go'
for path in (ready, go): os.mkfifo(path)
pipe = os.open(ready, os.O_RDWR|os.O_NONBLOCK)
script, output, errors, injected, joined, down = [W/('sync-error.'+suffix) for suffix in ('gdb','out','err','injected','joined','down')]
debug = None; handles = []
try:
    C.write_text(C.read_text()+f'routes={{sync="127.0.0.1:{server.server_port}"}}\n'); run(['start'])
    hook = '#!/bin/sh\nexec curl -fsS http://sync/hold\n'
    run(['--', 'sh', '-c', 'mkdir -p .git/hooks; printf %s "$1" >.git/hooks/post-merge; chmod +x .git/hooks/post-merge', 'sh', hook]); run(['stop'])
    (P/'changed').write_text('CHANGED\n'); git(P, 'add', 'changed'); git(P, '-c', 'user.name=user', '-c', 'user.email=user@pinfold.invalid', 'commit', '-qm', 'changed')
    script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
handle SIGINT stop nopass
python
import gdb,json,os,pathlib,signal,hashlib
starter=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:self.enabled=False;gdb.execute('set follow-fork-mode parent')
  return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main')
end
run -- printf MUST-NOT-RUN > {shlex.quote(str(output))} 2> {shlex.quote(str(errors))}
python
owner=gdb.selected_inferior().pid;assert starter is not None and owner!=starter,'interrupt did not stop the detached owner'
loc=json.loads(pathlib.Path({str(locator(P))!r}).read_text());claim=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');record=json.loads(claim.read_text())
vmm=int((pathlib.Path(record['state_dir'])/'vmm').read_text().split()[0]);helpers=set()
for task in pathlib.Path('/proc/'+str(owner)+'/task').iterdir():
 for pid in task.joinpath('children').read_text().split():
  pid=int(pid)
  if pid==vmm:continue
  args=pathlib.Path('/proc/'+str(pid)+'/cmdline').read_bytes().split(b'\\0')
  if args[1:3]==[b'box',b'exec'] and args[3]==loc['name'].encode():helpers.add(pid)
assert len(helpers)==1,'required real initial-sync helper absent/ambiguous'
helper=helpers.pop();assert os.getpgid(helper)==helper,'sync helper lacks owned process group';os.kill(helper,signal.SIGSTOP)
class Joined(gdb.Breakpoint):
 def stop(self):
  if not pathlib.Path({str(injected)!r}).exists():return False
  assert not pathlib.Path('/proc/'+str(helper)).exists(),'sync helper not reaped after cancellation/join'
  self.enabled=False;pathlib.Path({str(joined)!r}).touch();return False
class Down(gdb.Breakpoint):
 def stop(self):
  if not pathlib.Path({str(injected)!r}).exists():return False
  assert pathlib.Path({str(joined)!r}).exists() and not pathlib.Path('/proc/'+str(helper)).exists(),'down preceded sync join/reaping'
  self.enabled=False;pathlib.Path({str(down)!r}).touch();return False
Joined({joined_at!r});Down({down_at!r})
gdb.execute('catch syscall wait4',to_string=True);catch=gdb.breakpoints()[-1]
commands='commands '+str(catch.number)+'\\nsilent\\nif $rax == '+str(vmm)+'\\nset $rax = -10\\npython pathlib.Path('+repr({str(injected)!r})+').touch()\\ndisable '+str(catch.number)+'\\nend\\ncontinue\\nend'
gdb.execute(commands)
with open({str(ready)!r},'w') as stream:stream.write(json.dumps({{'owner':owner,'starter':starter,'vmm':vmm,'helper':helper}})+'\\n');stream.flush()
with open({str(go)!r},'rb') as stream:assert stream.read(1)==b'G'
os.kill(vmm,signal.SIGKILL)
end
continue
python
assert pathlib.Path({str(injected)!r}).exists(),'real wait4 return was not faulted'
assert pathlib.Path({str(joined)!r}).exists() and pathlib.Path({str(down)!r}).exists(),'sync join/down barriers absent'
end
''')
    debug = sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B], cwd=P, env=E, stdout=sp.PIPE, stderr=sp.PIPE)
    assert receipt.wait(90), 'real initial Git sync did not reach HTTP barrier'
    debug.send_signal(signal.SIGINT)
    assert select.select([pipe],[],[],30)[0], 'required debugger/helper observation absent'
    observed = json.loads(os.read(pipe,65536)); handles = [os.pidfd_open(observed[key]) for key in ('owner','starter','vmm','helper')]
    with go.open('wb',buffering=0) as stream: stream.write(b'G')
    text, diagnostics = debug.communicate(timeout=100)
    assert debug.returncode == 0, (text+diagnostics)[-1200:]
    assert injected.exists() and joined.exists() and down.exists(), 'required fault/join/down markers missing'
    assert all(select.select([fd],[],[],30)[0] for fd in handles), 'owner, starter, VMM or sync helper remained alive'
    assert output.read_bytes() == b'' and errors.read_bytes(), 'CMD ran or startup error diagnostic absent'
    release.set(); run(['stop'])
    assert run(['--','printf','SYNC-ERROR-CONTROL']).stdout == b'SYNC-ERROR-CONTROL'; run(['stop'])
    emit('sync-ended-error: real HTTP-held sync helper SIGSTOP; actual VMM wait4 return replaced by -ECHILD; helper cancelled/reaped and worker joined before Box.down; CMD=absent; checked restart=SYNC-ERROR-CONTROL')
finally:
    release.set()
    if debug is not None and debug.poll() is None:
        for fd in handles:
            if not select.select([fd],[],[],0)[0]: signal.pidfd_send_signal(fd,signal.SIGKILL)
        debug.terminate()
        try: debug.communicate(timeout=15)
        except sp.TimeoutExpired: debug.kill(); debug.communicate(timeout=15)
    for fd in handles+[pipe]: os.close(fd)
    for args in (['stop'], ['disk','rm','pi-'+ident(P)+'-home'], ['disk','rm','pi-'+ident(P)+'-project']): cleanup_ok &= invoke(args).returncode == 0
    server.shutdown(); server.server_close(); thread.join()
    emit(f'cleanup own-boxes/disks={cleanup_ok} fixture=$W'); assert cleanup_ok
