#!/usr/bin/env python3
"""Mac real disk-preparation helpers: disks.py BIN IMAGE [fill|volume|all]."""
import ctypes,hashlib,json,os,pathlib,select,signal,subprocess as sp,sys,tempfile,time
assert sys.platform=='darwin','Apple container case'
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];MODE=sys.argv[3] if len(sys.argv)>3 else 'all'
W=pathlib.Path(tempfile.mkdtemp(prefix='pfrr-disk.',dir='/tmp')).resolve();HOME=str(pathlib.Path.home())
E=dict(PATH=os.environ.get('PATH','/usr/bin:/bin'),HOME=HOME,XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),
 XDG_CACHE_HOME=os.environ.get('XDG_CACHE_HOME',HOME+'/.cache'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
def emit(s):print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(HOME,'/Users/user'),flush=True)
def run(p,args,code=0):
 r=sp.run([B,*args],cwd=p,env=E,input=b'',capture_output=True,timeout=650)
 assert r.returncode==code,(args,r.returncode,r.stdout[:100],r.stderr[:300]);return r

def rows():
 result=[]
 for line in sp.run(['ps','-axo','pid=,ppid=,command='],capture_output=True,check=True,timeout=5).stdout.decode().splitlines():
  words=line.split(None,2)
  if len(words)==3:result.append((int(words[0]),int(words[1]),words[2]))
 return result

def note(queue,pid):
 queue.control([select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXIT|select.KQ_NOTE_EXEC)],0,0)

def wait_dead(queue,expected,timeout):
 deadline=time.monotonic()+timeout;observed=set()
 while observed!=expected:
  left=deadline-time.monotonic();assert left>0,'owner/helper survived teardown bound'
  for event in queue.control([],len(expected),left):
   if event.fflags&select.KQ_NOTE_EXIT and event.ident in expected:observed.add(event.ident)
 return observed

def case(kind,action):
 p=W/(kind+'-'+action);p.mkdir();ident=p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
 name='probe-rereview-'+ident;disks=[name+f'-disk{i}' for i in range(1 if kind=='fill' else 12)]
 attachments=[dict(name=disk,guest=f'/d{i}',size='1G') for i,disk in enumerate(disks)]
 if kind=='fill':
  # Pi deliberately has no disk.from tree; exercise the shared core through box up.
  seed=p/'seed';seed.mkdir();(seed/'marker').write_bytes(b'DISK-CONTROL')
  with (seed/'large-fill').open('wb') as output:
   for _ in range(64):output.write(b'F'*(1024*1024))
  attachments[0]['from']=str(seed)
 spec=dict(name=name,image=IMAGE,cpus=2,memory='1G',disks=attachments)
 def up():
  child=sp.Popen([B,'box','up'],cwd=p,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0)
  child.stdin.write(json.dumps(spec).encode()+b'\n');child.stdin.flush();return child
 starter=up();owner=starter.pid;queue=select.kqueue();note(queue,owner)
 native=None;seen=set();tracked=set();paused=False;control_child=None
 lib=ctypes.CDLL('/usr/lib/libproc.dylib');ids=(ctypes.c_int*256)()
 try:
  deadline=time.monotonic()+120
  while native is None:
   assert time.monotonic()<deadline,'required actual '+kind+' helper not observed'
   candidates=[]
   ids=(ctypes.c_int*256)();lib.proc_listchildpids(owner,ids,ctypes.sizeof(ids))
   pending=set(pid for pid in ids if pid and pid not in tracked)
   for pid in pending:
    try:note(queue,pid);tracked.add(pid)
    except ProcessLookupError:pass
   pending.update(event.ident for event in queue.control([],64,0) if event.ident!=owner and event.fflags&select.KQ_NOTE_EXEC)
   for pid in pending:
    try:os.kill(pid,signal.SIGSTOP)
    except ProcessLookupError:continue
    path=ctypes.create_string_buffer(4096);lib.proc_pidpath(pid,path,len(path))
    if not path.value.endswith(b'/cli/container'):
     try:os.kill(pid,signal.SIGCONT)
     except ProcessLookupError:pass
     continue
    data=sp.run(['ps','-p',str(pid),'-o','ppid=,command='],capture_output=True,timeout=5).stdout.decode().strip().split(None,1)
    if len(data)!=2 or int(data[0])!=owner:
     try:os.kill(pid,signal.SIGCONT)
     except ProcessLookupError:pass
     continue
    candidates.append((pid,int(data[0]),data[1]))
   for pid,ppid,args in candidates:
    seen.add(args)
    matched=('container run ' in args and 'init setup ' in args and ' --fill ' in args) if kind=='fill' else 'container volume ' in args
    if not matched:
     os.kill(pid,signal.SIGCONT);continue
    note(queue,pid);paused=True;native=pid
    stopping=time.monotonic()+2
    while True:
     state=sp.run(['ps','-o','state=','-p',str(pid)],capture_output=True,check=True,timeout=5).stdout
     if b'T' in state:break
     assert time.monotonic()<stopping,'required real helper did not stop'
    claim=H/(hashlib.sha256(('box:'+name).encode()).hexdigest()+'.owner')
    assert claim.exists(),'required disk-preparation claim absent'
    break
   if starter.poll() is not None:
    out,err=starter.communicate();raise AssertionError(('startup ended before required helper barrier',starter.returncode,out,err,sorted(seen)))
  interrupted=time.monotonic()
  if action=='stop':run(p,['box','down',name])
  else:os.kill(owner,signal.SIGTERM)
  out,err=starter.communicate(timeout=650)
  assert starter.returncode==0,(starter.returncode,out,err[:300])
  events=[json.loads(line) for line in out.splitlines()]
  assert len(events)==1 and events[0]['event']=='down' and events[0]['reason']=='signal',events
  wait_dead(queue,{owner,native},max(1,610-(time.monotonic()-interrupted)));paused=False
  elapsed=time.monotonic()-interrupted;assert elapsed<610,'preparation exceeded two 300-second cleanup phases'
  assert not claim.exists(),'completed interrupted preparation retained ownership'
  control_child=up();assert select.select([control_child.stdout],[],[],180)[0],'restart readiness absent'
  event=json.loads(control_child.stdout.readline());assert event['event']=='ready',event
  if kind=='fill':
   assert run(p,['box','exec',name,'--','cat','/d0/marker']).stdout==b'DISK-CONTROL'
   assert run(p,['box','exec',name,'--','stat','-c','%s','/d0/large-fill']).stdout.strip()==b'67108864'
  else:assert run(p,['box','exec',name,'--','printf','DISK-CONTROL']).stdout==b'DISK-CONTROL'
  run(p,['box','down',name]);control_child.communicate(timeout=650);assert control_child.returncode==0
  emit(f'{kind}-{action}: boundary=public-box-up; actual native helper SIGSTOP confirmed; owner-client=0 down/signal ready=absent; owner/helper kernel NOTE_EXIT; claim=absent; seconds={elapsed:.2f}; same-name restart=0/DISK-CONTROL'+('; full64MiB fill=correct' if kind=='fill' else '; attached-own-disks=12'))
 finally:
  if paused and native is not None:
   try:os.kill(native,signal.SIGCONT)
   except ProcessLookupError:pass
  for child in (starter,control_child):
   if child is not None and child.poll() is None:child.terminate();child.communicate(timeout=650)
  queue.close();run(p,['box','down',name])
  for disk in disks:run(p,['disk','rm',disk])
  emit(f'{kind}-{action}: own boxes/disks removed; fixture='+str(p))

for kind in ('fill','volume'):
 if MODE not in ('all',kind):continue
 for action in ('stop','signal'):case(kind,action)
emit('Preparation cleanup: helper removal and disk undo each bounded at 300 seconds; no image or shared resource removed')
