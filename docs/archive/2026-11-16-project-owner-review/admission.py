#!/usr/bin/env python3
"""Real-binary admission evidence: admission.py BIN IMAGE [peers|maintenance|all]."""
import fcntl, hashlib, json, os, pathlib, select, shlex, signal, subprocess as sp, sys, tempfile, time
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];MODE=sys.argv[3] if len(sys.argv)>3 else 'all'
R=pathlib.Path(__file__).resolve().parents[3];W=pathlib.Path(tempfile.mkdtemp(prefix='pfr-admit.',dir='/tmp')).resolve();P=W/'p';P.mkdir()
HOST_HOME=str(pathlib.Path.home());E=dict(PATH=os.environ.get('PATH','/usr/bin:/bin'),HOME=HOST_HOME,
 XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),XDG_CACHE_HOME=os.environ.get('XDG_CACHE_HOME',str(pathlib.Path(HOST_HOME)/'.cache')),
 PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true',GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL='/dev/null')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}';ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
L=H/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project');SOURCE=R/'crates/pinfold/src/pi/session.rs'
children=[];debugs=[]
def emit(s):print(str(s).replace(str(W),'$W').replace(str(R),'$REPO').replace(B,'$BIN').replace(HOST_HOME,'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,code=0):
 r=sp.run([B,*args],cwd=P,env=E,input=b'',capture_output=True,timeout=120)
 assert r.returncode==code,(args,r.returncode,r.stderr[:300]);return r

def anchor(section,text):
 lines=SOURCE.read_text().splitlines();start=next(i for i,s in enumerate(lines) if section in s)
 return f'{SOURCE}:{next(i+1 for i,s in enumerate(lines) if i>=start and text in s)}'

def debugger(label,args,location,follow=False,stdin='/dev/null',stdout=None):
 ready,go=W/(label+'.ready'),W/(label+'.go');os.mkfifo(ready);os.mkfifo(go)
 rf=os.open(ready,os.O_RDWR|os.O_NONBLOCK);gf=os.open(go,os.O_RDWR|os.O_NONBLOCK)
 result=W/(label+'.result');commands=W/(label+'.gdb');out=stdout or W/(label+'.out');err=W/(label+'.err')
 commands.write_text(f'''set pagination off
set confirm off
set breakpoint pending on
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
handle SIGTERM nostop noprint pass
python
import gdb,json,pathlib
starter=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:gdb.execute('set follow-fork-mode parent');self.enabled=False
  return False
class Gate(gdb.Breakpoint):
 def stop(self):
  pid=gdb.selected_inferior().pid
  if {follow!r} and (starter is None or pid==starter):return False
  self.enabled=False
  with open({str(ready)!r},'w') as f:f.write(json.dumps({{'pid':pid,'starter':starter}})+'\\n');f.flush()
  with open({str(go)!r},'rb') as f:assert f.read(1)==b'G'
  return False
if {follow!r}:
 Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main')
Gate({location!r})
def ended(event):pathlib.Path({str(result)!r}).write_text(json.dumps({{'exit':getattr(event,'exit_code',None)}}))
gdb.events.exited.connect(ended)
end
run {shlex.join(args)} < {shlex.quote(str(stdin))} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
''')
 p=sp.Popen(['gdb','-nx','-q','-batch','-x',str(commands),'--args',B],cwd=P,env=E,stdout=sp.PIPE,stderr=sp.PIPE)
 d=dict(process=p,ready=rf,go=gf,result=result,released=False);debugs.append(d)
 assert select.select([rf],[],[],120)[0],'required debugger barrier absent'
 d['observed']=json.loads(os.read(rf,4096));return d

def release(d):
 if not d['released']:os.write(d['go'],b'G');d['released']=True

def finish(d,code):
 out,err=d['process'].communicate(timeout=120)
 assert d['process'].returncode==0 and d['result'].exists(),(out+err)[-700:]
 assert json.loads(d['result'].read_text())['exit']==code,'inferior status mismatch'

def maintenance_case():
 run(['stop']);path=H/(hashlib.sha256(('project:'+ID).encode()).hexdigest()+'.lock')
 with path.open('rb') as lock:
  fcntl.flock(lock,fcntl.LOCK_EX)
  d=debugger('maintenance',['stop'],anchor('async fn maintenance(','let deadline ='))
  os.kill(d['observed']['pid'],signal.SIGTERM);release(d);finish(d,143)
  started=time.monotonic();r=run(['stop'],125);elapsed=time.monotonic()-started
  assert b'owner-timeout' in r.stderr and 8<=elapsed<=30 and not L.exists(),(elapsed,r.stderr)
  emit(f'maintenance: exclusive lock held; signal after installed-handler barrier=143; stop=125/owner-timeout seconds={elapsed:.2f}; no locator created')
 assert run(['stop']).returncode==0;assert run(['--','printf','MAINTENANCE-CONTROL']).stdout==b'MAINTENANCE-CONTROL'
 emit('maintenance-control: released lock; stop=0; command=0/MAINTENANCE-CONTROL')

def peers_case():
 inp,out=W/'guest.in',W/'guest.out';os.mkfifo(inp);os.mkfifo(out)
 inf=os.open(inp,os.O_RDWR);outf=os.open(out,os.O_RDWR|os.O_NONBLOCK)
 def line():
  assert select.select([outf],[],[],120)[0],'guest witness marker absent'
  data=bytearray()
  while not data.endswith(b'\n'):
   assert select.select([outf],[],[],120)[0], 'guest line truncated'
   data.extend(os.read(outf,1))
  return bytes(data)
 d=None
 try:
  script='echo WITNESS-READY; while IFS= read -r x; do case "$x" in ping) echo WITNESS-PONG;; done) echo WITNESS-DONE; break;; esac; done'
  d=debugger('queued-peer',['--','sh','-c',script],anchor('async fn serve(','accepted = listener.accept() => {'),True,inp,out)
  observed=json.loads(L.read_text());assert line()==b'WITNESS-READY\n'
  socket=str(H/('r-'+observed['generation'][:32])/'c.sock')
  queued='import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print("CONNECTED",flush=True); sys.stdin.read()'
  q=sp.Popen(['python3','-c',queued,socket],env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);children.append(q)
  assert q.stdout.readline()==b'CONNECTED\n';q.kill();q.communicate(timeout=10);assert q.returncode==-signal.SIGKILL
  root='''import os,socket,sys
assert os.geteuid()==0
s=socket.socket(socket.AF_UNIX);s.settimeout(30);s.connect(sys.argv[1]);print('ROOT-CONNECTED',flush=True)
try: answer=s.recv(1)
except ConnectionResetError: answer=b''
assert answer==b'', 'wrong-uid peer received a protocol reply'
print('ROOT-PEER-REFUSED',flush=True)
'''
  wrong=sp.Popen(['sudo','-n','python3','-c',root,socket],env=E,stdout=sp.PIPE,stderr=sp.PIPE);children.append(wrong)
  assert wrong.stdout.readline()==b'ROOT-CONNECTED\n';release(d)
  text,errors=wrong.communicate(timeout=60);assert wrong.returncode==0 and text==b'ROOT-PEER-REFUSED\n',(wrong.returncode,text,errors[:300])
  os.write(inf,b'ping\n');assert line()==b'WITNESS-PONG\n'
  assert run(['--','printf','ADMISSION-CONTROL']).stdout==b'ADMISSION-CONTROL';assert json.loads(L.read_text())==observed
  emit('peers: owner held before accept; queued connector=SIGKILL; actual root uid0 peer=EOF/refused; existing command=WITNESS-PONG; valid joined command=0/ADMISSION-CONTROL; same generation')
  os.write(inf,b'done\n');assert line()==b'WITNESS-DONE\n';finish(d,0)
  assert not L.exists(),'last witness departure did not finish teardown'
 finally:
  if d:release(d)
  os.close(inf);os.close(outf)

def mac_peers_case():
 run(['start']);observed=json.loads(L.read_text())
 script='echo WITNESS-READY; while IFS= read -r x; do case "$x" in ping) echo WITNESS-PONG;; done) echo WITNESS-DONE; break;; esac; done'
 w=sp.Popen([B,'--','sh','-c',script],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0);children.append(w)
 def line():
  assert select.select([w.stdout],[],[],120)[0],'Mac witness marker absent'
  result=w.stdout.readline();assert result,'Mac witness ended before marker';return result
 assert line()==b'WITNESS-READY\n'
 listing=[json.loads(line) for line in run(['box','list','--label','dev.pinfold.owner']).stdout.splitlines()]
 owner=next(row['owner'] for row in listing if row['name']==observed['name'] and row['labels']['dev.pinfold.generation']==observed['generation'])
 assert w.poll() is None and json.loads(L.read_text())==observed
 paused=False;started=time.monotonic()
 try:
  os.kill(owner,signal.SIGSTOP);paused=True
  while True:
   state=sp.run(['ps','-o','stat=','-p',str(owner)],capture_output=True,timeout=.5).stdout.strip()
   if b'T' in state:break
   assert time.monotonic()-started<.5,'Mac owner stop was not observed'
  socket=str(H/('r-'+observed['generation'][:32])/'c.sock')
  queued='import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print("CONNECTED",flush=True); sys.stdin.read()'
  q=sp.Popen(['python3','-c',queued,socket],env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0);children.append(q)
  assert select.select([q.stdout],[],[],.5)[0] and q.stdout.readline()==b'CONNECTED\n','Mac queued connection absent'
  q.kill();q.communicate(timeout=.5);assert q.returncode==-signal.SIGKILL
  os.kill(owner,signal.SIGCONT);paused=False;elapsed=time.monotonic()-started
  assert elapsed<1,'Mac owner pause exceeded one second'
 finally:
  if paused:os.kill(owner,signal.SIGCONT)
 w.stdin.write(b'ping\n');w.stdin.flush();assert line()==b'WITNESS-PONG\n'
 assert run(['--','printf','ADMISSION-CONTROL']).stdout==b'ADMISSION-CONTROL';assert json.loads(L.read_text())==observed
 w.stdin.write(b'done\n');w.stdin.close();w.stdin=None;assert line()==b'WITNESS-DONE\n';out,err=w.communicate(timeout=120)
 assert w.returncode==0 and not out,(w.returncode,out,err[:300]);assert json.loads(L.read_text())==observed
 emit(f'Mac peers: full-generation owner SIGSTOP confirmedT; pause={elapsed:.3f}s; queued connector=SIGKILL before accept; resumed same-generation witness=PONG; valid join=0/ADMISSION-CONTROL; witness exit=0; accept errno=not observed')
 emit('INSPECTED Mac wrong-UID peer and maintenance signal barrier: actual root-peer rejection and GDB lock barrier ran on Linux; no Mac privilege or debugger settings changed')

sp.run(['git','init','-q',str(P)],env=E,check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],env=E,check=True)
run(['config','edit']);c=W/'c/pinfold/projects'/ID/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
 assert MODE in ('all','peers','maintenance')
 if sys.platform=='linux':
  if MODE in ('all','maintenance'):maintenance_case()
  if MODE in ('all','peers'):peers_case()
 else:
  assert sys.platform=='darwin'
  if MODE in ('all','peers'):mac_peers_case()
  else:emit('INSPECTED Mac maintenance: deterministic installed-handler/lock barrier requires the Linux GDB case; no debugger settings changed')
finally:
 for d in debugs:release(d)
 for p in children:
  if p.poll() is None:p.kill();p.communicate(timeout=30)
 try:run(['stop'])
 finally:
  for d in debugs:
   if d['process'].poll() is None:d['process'].terminate();d['process'].communicate(timeout=30)
   os.close(d['ready']);os.close(d['go'])
  run(['disk','rm','--all'])
 emit('fixture=$W; scoped cleanup complete')
