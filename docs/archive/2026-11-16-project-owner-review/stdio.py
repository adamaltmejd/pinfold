#!/usr/bin/env python3
"""Manual real-binary check: stdio.py BIN IMAGE; shared caller descriptions."""
import fcntl, hashlib, json, os, pathlib, resource, select, subprocess as sp, sys, tempfile
B=str(pathlib.Path(sys.argv[1]).resolve()); IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfr-stdio.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
HOST_HOME=str(pathlib.Path.home()); E=dict(PATH=os.environ.get('PATH','/usr/bin:/bin'),HOME=HOST_HOME,
 XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),XDG_CACHE_HOME=os.environ.get('XDG_CACHE_HOME',str(pathlib.Path(HOST_HOME)/'.cache')),
 PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true',GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL='/dev/null')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
children=[]
def emit(s): print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(HOST_HOME,'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def git(*a): return sp.run(['git','-C',str(P),*a],env=E,check=True,capture_output=True)
def run(args):
 r=sp.run([B,*args],cwd=P,env=E,input=b'',capture_output=True,timeout=120)
 assert r.returncode==0,(args,r.returncode,r.stderr[:200]); return r
ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'; L=H/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project')
N=131072
script='echo READY >&2; IFS= read -r x; head -c "$1" /dev/zero | tr "\\000" "$2"; echo DONE >&2'
def pair(fd):
 clients=[]
 for label in ('A','B'):
  p=sp.Popen([B,'--','sh','-c',script,'sh',str(N),label],cwd=P,env=E,stdin=sp.PIPE,stdout=fd,stderr=sp.PIPE,bufsize=0)
  children.append(p); clients.append(p)
 for p in clients:
  notices=0
  while True:
   ready=select.select([p.stderr],[],[],120)[0]
   assert ready, f'guest barrier absent: pid={p.pid} status={p.poll()} notices={notices}'
   line=p.stderr.readline()
   if line==b'READY\n': break
   if b'waiting for the project box to start' in line:
    notices+=1; assert notices==1,'repeated waiting notice'; continue
   emit(f'guest barrier unexpected stderr={line!r} pid={p.pid} status={p.poll()} notices={notices}')
   raise AssertionError('guest barrier replaced by stderr or EOF')
  emit(f'guest READY status={p.poll()} waiting-notices={notices}')
 for p in clients: p.stdin.write(b'GO\n');p.stdin.close();p.stdin=None
 return clients

def finish(clients):
 for p in clients:
  out,err=p.communicate(timeout=120)
  assert p.returncode==0 and err==b'DONE\n',(p.returncode,err[:200])
 assert json.loads(L.read_text())==owner,'joined generation changed'

def check(data): assert len(data)==2*N and data.count(b'A')==N and data.count(b'B')==N,'binary output lost/interleaved incorrectly'

git('init','-q');git('-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture')
run(['config','edit']);c=W/'c/pinfold/projects'/ID/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
 run(['start']);owner=json.loads(L.read_text());assert run(['--','printf','CONTROL']).stdout==b'CONTROL'
 emit('control: one joined command=0 output=CONTROL')
 read,write=os.pipe()
 try:
  pristine=fcntl.fcntl(write,fcntl.F_GETFL);os.write(write,b'PRIME');assert os.read(read,5)==b'PRIME'
  before=fcntl.fcntl(write,fcntl.F_GETFL);emit(f'parent pipe write-control: pristine={pristine:#x} after-parent-write={before:#x}')
  clients=pair(write);data=bytearray()
  while len(data)<2*N:
   assert select.select([read],[],[],120)[0],'shared pipe output stalled'
   part=os.read(read,2*N-len(data));assert part,'shared pipe truncated';data.extend(part)
  finish(clients);after=fcntl.fcntl(write,fcntl.F_GETFL);check(data)
  emit(f'shared-pipe raw flags: before={before:#x} after={after:#x} O_NONBLOCK={os.O_NONBLOCK:#x}')
  assert before==after and not after&os.O_NONBLOCK,'shared pipe flags changed'
  emit(f'shared-pipe: clients=2 exits=0,0 bytes={len(data)} A={N} B={N} before={before} after={after} nonblocking=False same-generation=True')
 finally: os.close(read);os.close(write)
 path=W/'shared-output';fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
 try:
  os.write(fd,b'PRIME');os.lseek(fd,0,os.SEEK_SET);os.ftruncate(fd,0)
  before=fcntl.fcntl(fd,fcntl.F_GETFL);clients=pair(fd);finish(clients)
  after=fcntl.fcntl(fd,fcntl.F_GETFL);data=path.read_bytes();check(data)
  emit(f'shared-file raw flags: before={before:#x} after={after:#x} O_NONBLOCK={os.O_NONBLOCK:#x}')
  assert before==after and not after&os.O_NONBLOCK,'shared file flags changed'
  emit(f'shared-file: clients=2 exits=0,0 bytes={len(data)} A={N} B={N} before={before} after={after} nonblocking=False same-generation=True')
 finally: os.close(fd)
 assert run(['--','printf','AFTER']).stdout==b'AFTER';emit('control: same-owner post-concurrency command=0 output=AFTER')
 run(['stop']);old_owner=owner;held=[];edges=[];limits=resource.getrlimit(resource.RLIMIT_NOFILE)
 assert limits[1]==resource.RLIM_INFINITY or limits[1]>=9000,'sparse-fd fixture requires hard limit >=9000'
 resource.setrlimit(resource.RLIMIT_NOFILE,(max(limits[0],9000),limits[1]))
 emit(f'descriptor fixture: original-soft={limits[0]} run-soft={max(limits[0],9000)} scope=driver-and-children-only')
 try:
  for _ in range(300):held.append(os.open('/dev/null',os.O_RDONLY|os.O_CLOEXEC))
  for args,expected in ((['start'],b''),(['--','printf','FD-CONTROL'],b'FD-CONTROL')):
   read,write=os.pipe();edges.extend((read,write));high=fcntl.fcntl(write,fcntl.F_DUPFD_CLOEXEC,8192);edges.append(high)
   p=sp.Popen([B,*args],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE,pass_fds=(*held,high));children.append(p)
   os.close(write);edges.remove(write);os.close(high);edges.remove(high)
   out,err=p.communicate(timeout=120);assert p.returncode==0 and out==expected,(args,p.returncode,out,err[:300])
   assert select.select([read],[],[],5)[0] and os.read(read,1)==b'','passed descriptor retained its pipe writer'
   observed=json.loads(L.read_text())
   if args==['start']:assert observed!=old_owner;owner=observed
   else:assert observed==owner
   emit(f'many-fd {args[0]}: passed=301 sparse={high} exit=0 stdout={out!r} unused-pipe EOF=True owner=kept; same generation for join')
   os.close(read);edges.remove(read)
 finally:
  for fd in held+edges:os.close(fd)
  resource.setrlimit(resource.RLIMIT_NOFILE,limits)
 assert run(['--','printf','AFTER-FDS']).stdout==b'AFTER-FDS' and json.loads(L.read_text())==owner
 emit('control: post-descriptor same-generation command=0/AFTER-FDS; observed EOF only, not complete authority-leak stress')
finally:
 for p in children:
  if p.poll() is None:p.kill();p.communicate(timeout=30)
 for args in (['stop'],['disk','rm','--all']):
  try:run(args)
  except Exception as e:emit(f'cleanup {args[0]} failed={e}');raise
 emit('fixture=$W; scoped cleanup complete')
