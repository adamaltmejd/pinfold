#!/usr/bin/env python3
"""Linux startup evidence barriers: startup.py BIN IMAGE; no sleeps."""
import fcntl,hashlib,http.server,json,os,pathlib,select,shlex,signal,subprocess as sp,sys,tempfile,threading,time
assert sys.platform.startswith('linux')
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfou.',dir='/tmp')).resolve();P=W/'p';P.mkdir();H=pathlib.Path('/tmp')/f'pinfold-{os.getuid()}'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
def emit(s):print(str(s).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user').replace(B,'$BIN'),flush=True)
def run(args,code=0):
 r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=45)
 assert r.returncode==code,(args,r.returncode,r.stderr[:300]);return r
sp.run(['git','init','-q',str(P)],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
C=W/'c/pinfold/projects'/ID/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
KEY=hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest();L=H/(KEY+'.project');LOCK=H/(hashlib.sha256(('project-launch:'+KEY).encode()).hexdigest()+'.lock')
def locked():
 with LOCK.open('rb') as f:
  try:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
  except BlockingIOError:return True
  return False

def case(label,location,kill_starter=False):
 ready=W/(label+'.ready');go=W/(label+'.go');os.mkfifo(ready);os.mkfifo(go)
 pipe=os.open(ready,os.O_RDWR|os.O_NONBLOCK);commands=W/(label+'.gdb');out=W/(label+'.out');err=W/(label+'.err')
 commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,pathlib
starter=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False
  gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:
   gdb.execute('set follow-fork-mode parent');self.enabled=False
  return False
class Gate(gdb.Breakpoint):
 def stop(self):
  owner=gdb.selected_inferior().pid
  if starter is None or owner==starter:return False
  self.enabled=False
  with open({str(ready)!r},'w') as f:f.write(json.dumps({{'starter':starter,'owner':owner}})+'\\n');f.flush()
  with open({str(go)!r},'rb') as f:assert f.read(1)==b'G','gate cancelled'
  return False
Spawn('pinfold::pi::session::spawn_owner')
Main('pinfold::main')
Gate({location!r})
end
run start > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
''')
 debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(commands),'--args',B],cwd=P,env=E,stdout=sp.PIPE,stderr=sp.PIPE);owner_fd=None
 try:
  assert select.select([pipe],[],[],90)[0],'required startup gate not hit'
  observed=json.loads(os.read(pipe,4096));owner_fd=os.pidfd_open(observed['owner']);before=json.loads(L.read_text());assert locked(),'launch ownership absent at gate'
  if kill_starter:
   assert before['generation'] is not None,'keep gate precedes generation publication'
   starter_fd=os.pidfd_open(observed['starter']);signal.pidfd_send_signal(starter_fd,signal.SIGKILL);assert select.select([starter_fd],[],[],30)[0];os.close(starter_fd)
   assert locked() and json.loads(L.read_text())==before,'starter death released launch or changed locator'
  else:
   assert before['generation'] is None,'preclaim gate already published generation'
   name_record=H/(hashlib.sha256(('box:'+before['name']).encode()).hexdigest()+'.owner');assert not name_record.exists()
   started=time.monotonic();refused=run(['stop'],125);elapsed=time.monotonic()-started
   assert b'owner-timeout' in refused.stderr and 8<=elapsed<=15,(elapsed,refused.stderr)
   assert locked() and json.loads(L.read_text())==before and not name_record.exists(),'timeout changed live bootstrap ownership'
   emit(f'preclaim-stop: gate=after-locator/before-claim stop=125/owner-timeout seconds={elapsed:.2f} locator=unchanged launch=held')
  with go.open('wb',buffering=0) as f:f.write(b'G')
  run(['start']);after=json.loads(L.read_text());assert after['name']==before['name'] and after['generation'] is not None
  if kill_starter:assert after==before,'lost keep acknowledgement discarded committed keep'
  assert run(['--','sh','-c','printf STARTUP-CONTROL']).stdout==b'STARTUP-CONTROL';run(['stop']);assert not L.exists()
  output,errors=debug.communicate(timeout=90);assert debug.returncode==0,(output+errors)[-800:]
  emit(label+': required gate hit; repeated start=0 marker=STARTUP-CONTROL same-owner=True stop=0'+(' starter=SIGKILL inherited-launch=held lost-keep-ack=survived' if kill_starter else ''))
 finally:
  os.close(pipe)
  if debug.poll() is None:
   if owner_fd is not None and not select.select([owner_fd],[],[],0)[0]:signal.pidfd_send_signal(owner_fd,signal.SIGKILL)
   debug.terminate()
   try:debug.communicate(timeout=10)
   except sp.TimeoutExpired:debug.kill();debug.communicate(timeout=10)
  if owner_fd is not None:os.close(owner_fd)
def sync_stop():
 receipt=threading.Event();release=threading.Event()
 class Fixture(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   assert self.path=='/hold';receipt.set();assert release.wait(90),'fixture release absent'
   try:self.send_response(200);self.end_headers();self.wfile.write(b'SYNC-RELEASED')
   except BrokenPipeError:pass
  def log_message(self,*args):pass
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Fixture);thread=threading.Thread(target=server.serve_forever);thread.start();starter=None
 try:
  C.write_text(C.read_text()+f'routes = {{ "sync-probe" = "127.0.0.1:{server.server_port}" }}\n');run(['start'])
  hook='#!/bin/sh\nexec curl -fsS http://sync-probe/hold\n'
  run(['--','sh','-c','mkdir -p .git/hooks; printf %s "$1" >.git/hooks/post-merge; chmod +x .git/hooks/post-merge','sh',hook]);run(['stop'])
  (P/'sync-change').write_text('SYNC-CHANGE\n');sp.run(['git','-C',str(P),'add','sync-change'],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','sync-change'],check=True)
  starter=sp.Popen([B,'--','sh','-c','printf MUST-NOT-RUN'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
  assert receipt.wait(90),'real post-merge hook did not reach fixture barrier'
  before=json.loads(L.read_text());assert before['generation'] is not None,'HTTP receipt preceded core readiness'
  run(['stop']);output,errors=starter.communicate(timeout=45);assert starter.returncode==125 and not output,(starter.returncode,output,errors[:300])
  assert not L.exists(),'stop during sync left locator';release.set()
  assert run(['--','sh','-c','printf SYNC-CONTROL']).stdout==b'SYNC-CONTROL'
  assert run(['--','curl','-fsS','http://sync-probe/hold']).stdout==b'SYNC-RELEASED';run(['stop'])
  emit('sync-stop: real post-merge curl held by HTTP receipt; stop=0 starter=125 CMD=absent restart=SYNC-CONTROL allowed-route=SYNC-RELEASED')
 finally:
  release.set()
  if starter is not None and starter.poll() is None:starter.terminate();starter.communicate(timeout=30)
  try:run(['stop'])
  finally:server.shutdown();server.server_close();thread.join()
try:
 case('preclaim-control','pinfold::pi::state::record_run')
 case('lost-keep-ack','pinfold::pi::session::redirect_stderr',True)
 sync_stop()
 emit('NOT-RUN this driver: starter death during preparation/sync; lost Join acknowledgement')
 emit('PASS preclaim timeout, committed-Keep acknowledgement loss, and real initial-sync stop barriers')
finally:
 try:run(['stop'])
 except Exception as error:emit('cleanup-stop-failed='+str(error))
 try:run(['disk','rm','--all'])
 except Exception as error:emit('cleanup-disk-failed='+str(error))
 emit('fixture='+str(W))
