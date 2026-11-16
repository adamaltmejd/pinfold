#!/usr/bin/env python3
"""Linux real catalog/lease contention: startup-linux.py BIN IMAGE [catalog|lease|all]."""
import fcntl,hashlib,json,os,pathlib,select,shlex,signal,stat,subprocess as sp,sys,threading,time
assert sys.platform=='linux'
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
MODE=sys.argv[3] if len(sys.argv)>3 else 'all';ACTION=sys.argv[4] if len(sys.argv)>4 else 'all';R=pathlib.Path(__file__).resolve().parents[3]
P.mkdir();git(P,'init','-q','-b','review');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture')
run(['config','edit']);C=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
store=(pathlib.Path(E['XDG_CACHE_HOME'])/'pinfold/images').resolve();catalog=json.loads((store/'catalog.json').read_text())
image_id=catalog['references'][IMAGE];assert image_id.startswith('sha256:')
source=R/'crates/pinfold/src/core/image/linux/catalog.rs';lines=source.read_text().splitlines()
def location(needle):
 matches=[i+1 for i,line in enumerate(lines) if needle in line];assert len(matches)==1,(needle,matches);return str(source)+':'+str(matches[0])
waiting_line=location('match nix::poll::poll(&mut [], 20u16)')
lease_line=location('let lease = startup_lock(&key(&image.id)?')
key=hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest();launch=H/(hashlib.sha256(('project-launch:'+key).encode()).hexdigest()+'.lock')

def case(kind,action):
 label=kind+'-'+action;ready=W/(label+'.ready');go=W/(label+'.go');status_file=W/(label+'.status')
 for path in (ready,go):os.mkfifo(path)
 read=os.open(ready,os.O_RDWR|os.O_NONBLOCK);go_fd=os.open(go,os.O_RDWR|os.O_NONBLOCK|os.O_CLOEXEC);script=W/(label+'.gdb');out=W/(label+'.out');err=W/(label+'.err')
 target='pinfold::core::image::linux::catalog::resolve_leased' if kind=='catalog' else lease_line
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,threading,hashlib
starter=None
status_thread=None
def report(value):
 with open({str(ready)!r},'w') as stream:stream.write(json.dumps(value)+'\\n');stream.flush()
def status():
 pid,value=os.waitpid(starter,0)
 pathlib.Path({str(status_file)!r}).write_text(str(os.waitstatus_to_exitcode(value)))
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
  global status_thread
  owner=gdb.selected_inferior().pid
  if starter is None or owner==starter:return False
  self.enabled=False
  loc=json.loads(pathlib.Path({str(locator(P))!r}).read_text());claim=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');record=json.loads(claim.read_text())
  report({{'event':'gate','owner':owner,'starter':starter,'name':loc['name'],'generation':record['generation']}})
  status_thread=threading.Thread(target=status);status_thread.start()
  with open({str(go)!r},'rb') as stream:assert stream.read(1)==b'G'
  return False
class Waiting(gdb.Breakpoint):
 def stop(self):
  self.enabled=False;report({{'event':'waiting'}});return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Gate({target!r});Waiting({waiting_line!r})
end
run -- sh -c 'printf MUST-NOT-RUN' > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
if status_thread is not None:
 status_thread.join(90)
 assert not status_thread.is_alive(),'starter did not exit'
end
''')
 debuglog=W/(label+'.debug')
 with debuglog.open('wb') as log:
  debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=log,stderr=sp.STDOUT)
 pending=b'';observed=None;owner_fd=starter_fd=holder=None
 def message(timeout=90):
  nonlocal pending
  deadline=time.monotonic()+timeout
  while b'\n' not in pending:
   assert select.select([read],[],[],max(0,deadline-time.monotonic()))[0],'required debugger barrier absent'
   pending+=os.read(read,65536)
  value,pending=pending.split(b'\n',1);return json.loads(value)
 def release_gate():
  # Keep a reader open before the report, so release cannot lose its byte.
  os.write(go_fd,b'G')
 try:
  observed=message();assert observed['event']=='gate';owner_fd=os.pidfd_open(observed['owner']);starter_fd=os.pidfd_open(observed['starter'])
  suffix='catalog' if kind=='catalog' else image_id
  lock_key='images:'+hashlib.sha256(os.fsencode(store)).hexdigest()+':'+suffix;lock_path=H/(hashlib.sha256(lock_key.encode()).hexdigest()+'.lock')
  holder=os.open(lock_path,os.O_RDWR|os.O_CREAT|os.O_CLOEXEC|os.O_NOFOLLOW,0o600);metadata=os.fstat(holder)
  assert metadata.st_uid==os.getuid() and stat.S_ISREG(metadata.st_mode) and stat.S_IMODE(metadata.st_mode)==0o600
  fcntl.flock(holder,fcntl.LOCK_EX|fcntl.LOCK_NB);release_gate();assert message()['event']=='waiting'
  began=time.monotonic();assert not select.select([starter_fd],[],[],12)[0] and not status_file.exists(),'starter ended while live owner supplied progress'
  assert out.read_bytes()==b'' and b'owner-timeout' not in err.read_bytes();assert time.monotonic()-began>=12
  interrupted=time.monotonic()
  if action=='stop':run(['stop'])
  else:signal.pidfd_send_signal(starter_fd,signal.SIGTERM)
  assert select.select([owner_fd],[],[],90)[0],'owner survived cancellation while lock remained held'
  debug.communicate(timeout=100);assert debug.returncode==0,debuglog.read_bytes()[-1200:]
  elapsed=time.monotonic()-interrupted;expected=125 if action=='stop' else 143
  assert status_file.read_text()==str(expected) and out.read_bytes()==b'' and elapsed<10,(status_file.read_text(),err.read_bytes(),elapsed)
  if action=='stop':assert b'stopped' in err.read_bytes() and b'owner-timeout' not in err.read_bytes()
  assert not locator(P).exists() and not (H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')).exists()
  with open(lock_path,'rb') as probe:
   try:fcntl.flock(probe,fcntl.LOCK_EX|fcntl.LOCK_NB)
   except BlockingIOError:pass
   else:raise AssertionError('competing holder was not retained through owner exit')
  with launch.open('rb') as probe:fcntl.flock(probe,fcntl.LOCK_EX|fcntl.LOCK_NB)
  os.close(holder);holder=None
  assert run(['--','printf','LOCK-CONTROL']).stdout==b'LOCK-CONTROL';run(['stop'])
  emit(f'{label}: required lock-contention barrier; heartbeat-kept-starter=12s CMD=absent; launcher={expected} '+('stop=0/stopped; ' if action=='stop' else 'SIGTERM; ')+f'owner exited with competitor held; seconds={elapsed:.2f}; claim/locator=absent launch=available; restart=0/LOCK-CONTROL')
 finally:
  release_gate()
  if holder is not None:os.close(holder)
  if starter_fd is not None and not select.select([starter_fd],[],[],0)[0]:signal.pidfd_send_signal(starter_fd,signal.SIGTERM)
  if debug.poll() is None:
   debug.terminate()
   try:debug.communicate(timeout=10)
   except sp.TimeoutExpired:debug.kill();debug.communicate(timeout=10)
  for fd in (read,go_fd,owner_fd,starter_fd):
   if fd is not None:os.close(fd)
  run(['stop'])
try:
 for kind in ('catalog','lease'):
  if MODE in ('all',kind):
   for action in ('stop','signal'):
    if ACTION in ('all',action):case(kind,action)
finally:
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup exact own boxes/disks=True')
