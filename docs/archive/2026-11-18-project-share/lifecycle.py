"""Kill real Apple project owners at startup and native retirement barriers."""
import ctypes
from pathlib import Path
source=Path(__file__).with_name('apple.py')
exec(compile(source.read_text().split('\ntry:\n',1)[0],str(source),'exec'),globals())
H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
lib=ctypes.CDLL('/usr/lib/libproc.dylib');paused=set()
def note(q,pid):
 q.control([select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXIT|select.KQ_NOTE_EXEC)],0,0)
def resume(pid):
 try:os.kill(pid,signal.SIGCONT)
 except ProcessLookupError:pass
 paused.discard(pid)
def barrier(q,parent,match,limit=120,hold_parent=False):
 tracked=set();deadline=time.monotonic()+limit
 while time.monotonic()<deadline:
  ids=(ctypes.c_int*256)();lib.proc_listchildpids(parent,ids,ctypes.sizeof(ids))
  pending={pid for pid in ids if pid and pid not in tracked}
  for pid in pending:
   try:note(q,pid);tracked.add(pid)
   except ProcessLookupError:pass
  pending.update(e.ident for e in q.control([],64,0) if e.fflags&select.KQ_NOTE_EXEC)
  for pid in pending:
   if pid==parent:continue
   if hold_parent:os.kill(parent,signal.SIGSTOP);paused.add(parent)
   try:os.kill(pid,signal.SIGSTOP);paused.add(pid)
   except ProcessLookupError:
    if hold_parent:resume(parent)
    continue
   data=sp.run(['ps','-p',str(pid),'-o','ppid=,command='],capture_output=True,timeout=5).stdout.decode().strip().split(None,1)
   if len(data)==2 and int(data[0])==parent and match(data[1]):
    stopped=time.monotonic()+2
    observed=parent if hold_parent else pid
    while b'T' not in sp.run(['ps','-p',str(observed),'-o','state='],capture_output=True,timeout=5).stdout:
     assert time.monotonic()<stopped,'native helper did not stop'
    return pid
   resume(pid)
   if hold_parent:resume(parent)
 raise AssertionError('real native barrier not observed')
def death(q,pid):
 deadline=time.monotonic()+15
 while time.monotonic()<deadline:
  if any(e.ident==pid and e.fflags&select.KQ_NOTE_EXIT for e in q.control([],64,max(0,deadline-time.monotonic()))):return
 raise AssertionError('owner kernel exit not observed')
def locator(p):return H/(hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()+'.project')
def claim(name):return H/(hashlib.sha256(('box:'+name).encode()).hexdigest()+'.owner')
def replacement(p,old):
 # Hold the remover with native kill/stop observed, before it can launch rm.
 q=select.kqueue();c=spawn(p,'echo REPLACEMENT; cat >/dev/null');note(q,c.pid)
 try:
  native=barrier(q,c.pid,lambda a:'container kill --signal KILL '+old['name'] in a or 'container stop '+old['name'] in a,hold_parent=True)
  assert claim(old['name']).exists()
  boxes=listed(p);assert len(boxes)==1 and boxes[0]['name']==old['name']
  assert not select.select([c.stdout],[],[],0)[0] and c.poll() is None
  say('orphan-removal barrier before native rm: old-generation='+old['generation']+' replacement-command=absent replacement-box=absent claim=present')
  resume(native);resume(c.pid);assert line(c)=='REPLACEMENT'
  boxes=listed(p);assert len(boxes)==1
  assert boxes[0]['labels']['dev.pinfold.generation']!=old['generation']
  assert not claim(old['name']).exists();pf(p,'box','stat',old['name'],code=3)
  c.stdin.close();c.stdin=None;done(c)
  assert not listed(p);say('replacement followed checked old-box removal and claim retirement=True')
 finally:q.close()
try:
 p=setup('startup-death',False);q=select.kqueue()
 c=spawn(p,'echo MUST-NOT-RUN; cat >/dev/null');note(q,c.pid)
 try:
  owner=barrier(q,c.pid,lambda a:'__project-owner' in a);note(q,owner);resume(owner)
  native=barrier(q,owner,lambda a:'container exec --user 0:0 ' in a and 'init proxy-socket' in a)
  loc=json.loads(locator(p).read_text());record=json.loads(claim(loc['name']).read_text())
  old={'name':loc['name'],'generation':record['generation']}
  assert len(listed(p))==1 and not record['ready']
  os.kill(owner,signal.SIGKILL);death(q,owner);resume(native)
  out,err=c.communicate(timeout=180);say(f'startup-owner-killed client={c.returncode} command-bytes={len(out)} err={err.decode(errors="replace").strip()[:300]!r}')
  assert c.returncode==125 and not out
  replacement(p,old)
 finally:q.close()
 p=setup('retirement-death',False);c=spawn(p,'echo READY-TO-LEAVE; cat >/dev/null');line(c)
 box=listed(p)[0];owner=box['owner'];old={'name':box['name'],'generation':box['labels']['dev.pinfold.generation']}
 q=select.kqueue();note(q,owner)
 try:
  c.stdin.close();c.stdin=None
  # Freeze the owner with a kill/stop helper. Native rm has not been launched.
  native=barrier(q,owner,lambda a:'container kill --signal KILL '+old['name'] in a or 'container stop '+old['name'] in a,hold_parent=True)
  assert claim(old['name']).exists();os.kill(owner,signal.SIGKILL);death(q,owner)
  paused.discard(owner)
  # The orphan helper is ours; end it before allowing recovery to do the removal.
  try:os.kill(native,signal.SIGKILL)
  except ProcessLookupError:pass
  paused.discard(native)
  done(c);say('retirement-owner-killed: owner stopped with native kill/stop helper observed, before rm; completed status retained=True')
  replacement(p,old)
 finally:q.close()
 # Keep and final leave race without a gate; either serialized generation is valid.
 p=setup('keep-leave-race',False);c=spawn(p,'echo LEAVE; cat >/dev/null');line(c)
 k=sp.Popen([str(B),'start'],cwd=p,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE);clients.append(k)
 c.stdin.close();c.stdin=None;done(c);done(k);assert len(listed(p))==1
 pf(p,'stop');assert not listed(p);say('keep/final-leave race kept one generation; stop removed it=True')
 say('PASS startup and retirement owner death; no image build')
finally:
 for pid in list(paused):resume(pid)
 for c in clients:
  if c.poll() is None:c.terminate();c.communicate(timeout=180)
 for p in projects:
  try:pf(p,'stop')
  except Exception as error:say('cleanup stop error='+str(error))
 for row in pf(W,'disk','list','--json').stdout.splitlines():pf(W,'disk','rm',json.loads(row)['name'],'--json')
 assert not pf(W,'disk','list','--json').stdout.strip()
 shutil.rmtree(W);say('fixture-removed='+str(not W.exists()))
