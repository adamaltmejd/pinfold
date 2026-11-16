#!/usr/bin/env python3
"""Real unreadable/unknown claim controls: claims.py BIN IMAGE; no global cleanup."""
import hashlib,json,os,pathlib,select,signal,subprocess as sp,sys,tempfile
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];LINUX=sys.platform.startswith('linux');assert os.geteuid()!=0
W=pathlib.Path(tempfile.mkdtemp(prefix='pfocl.',dir='/tmp')).resolve();H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();seed=W/'seed';seed.mkdir();(seed/'marker').write_bytes(b'CLAIM-CONTROL\n')
prefix=f'probe-owner-claim-{os.getpid()}';A,BX,N,U,C,D=[prefix+'-'+x for x in ('a','b','new','unknown','dead','second-new')]
disk,fresh,unknown,second=[prefix+'-'+x for x in ('filled','fresh','unknown-disk','second-fresh')];children={};faults=[];command=None

def emit(s):print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(str(pathlib.Path.home()),'/home/user' if LINUX else '/Users/user'),flush=True)
def owner(name):return H/(hashlib.sha256(('box:'+name).encode()).hexdigest()+'.owner')
def run(args,code=0):
 r=sp.run([B]+args,cwd=W,env=E,input=b'',capture_output=True,timeout=180)
 emit(f'{args!r} exit={r.returncode} out={r.stdout[:180]!r} err={r.stderr[:300]!r}')
 if code is not None:assert r.returncode==code,'CLI status mismatch'
 return r

def line(p):
 assert select.select([p.stdout],[],[],180)[0],'readiness deadline'
 got=p.stdout.readline();assert got,'owner ended before readiness';return got

def start(name,attached=(),expected='ready'):
 spec={'name':name,'image':IMAGE,'cpus':2,'memory':'1G','disks':[{'name':d,'guest':f'/d{i}','size':'1G',**({'from':str(seed)} if d in (disk,unknown) else {})} for i,d in enumerate(attached)]}
 p=sp.Popen([B,'box','up'],cwd=W,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0);p.stdin.write(json.dumps(spec).encode()+b'\n');p.stdin.flush()
 event=json.loads(line(p));emit(f'up {name}: event={event["event"]} reason={event.get("reason")}')
 assert event['event']==expected,event
 if expected=='ready':children[name]=p
 else:
  out,err=p.communicate(timeout=180);assert p.returncode==1,(out,err);emit(f'refusal reason={event.get("reason")} detail={event.get("detail")}')
 return event

def down(name):
 run(['box','down',name]);p=children.pop(name,None)
 if p is not None:
  out,err=p.communicate(timeout=180);assert p.returncode==0,(out,err)

def backing(d):
 path=W/'s/pinfold/disks'/(d+'.ext4');meta=path.stat();digest=hashlib.sha256()
 with path.open('rb') as f:
  for block in iter(lambda:f.read(1<<20),b''):digest.update(block)
 return meta.st_ino,meta.st_size,digest.digest()

def dead(name):
 record=json.loads(owner(name).read_bytes());vmm=int((pathlib.Path(record['state_dir'])/'vmm').read_text().split()[0]);fd=os.pidfd_open(vmm)
 p=children.pop(name);p.kill();p.wait(timeout=30)
 try:assert select.select([fd],[],[],30)[0],'VMM did not end after owner death'
 finally:os.close(fd)
 for stream in (p.stdin,p.stdout,p.stderr):stream.close()
 assert p.returncode==-signal.SIGKILL;emit(f'{name}: owner SIGKILL and actual VMM pidfd death observed');return record

try:
 start(A,[disk])
 command=sp.Popen([B,'box','exec',A,'--','sh','-c','cat /d0/marker; sync; echo ACTIVE; read answer; test "$answer" = continue; cat /d0/marker; echo STILL-ACTIVE'],cwd=W,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0)
 assert line(command)==b'CLAIM-CONTROL\n' and line(command)==b'ACTIVE\n'
 initial=backing(disk) if LINUX else None;bad=owner(A);bad.chmod(0);faults.append(bad)
 refused=start(BX,[disk],expected='refused');assert refused['reason']=='disk-in-use' and bad.name in refused['detail']
 if LINUX:assert backing(disk)==initial;emit('unreadable claim: backing inode/size/full SHA256 unchanged across refusal')
 out,err=command.communicate(b'continue\n',timeout=30);assert command.returncode==0 and out==b'CLAIM-CONTROL\nSTILL-ACTIVE\n' and not err;command=None
 assert children[A].poll() is None;emit('unreadable claim: pre-existing guest marker/control continued, exit=0')
 start(N,[fresh]);assert run(['box','exec',N,'--','printf','NEW-ONLY']).stdout==b'NEW-ONLY';down(N)
 report=run(['disk','list','--json']);assert {json.loads(x)['name'] for x in report.stdout.splitlines()}=={disk,fresh}
 assert bad.name.encode() in report.stderr;emit('unreadable claim: new-only admission and disk reporting continued with record warning')
 bad.chmod(0o600);faults.remove(bad)
 refused=start(BX,[disk],expected='refused');assert refused['reason']=='disk-in-use' and A in refused['detail'] and '.owner' not in refused['detail']
 down(A);start(BX,[disk]);assert run(['box','exec',BX,'--','cat','/d0/marker']).stdout==b'CLAIM-CONTROL\n';down(BX)
 emit('readability repair retained live named-disk protection; identical B request worked after normal A down')
 if LINUX:
  start(U,[unknown]);run(['box','exec',U,'--','sync']);observed=dead(U);saved=owner(U).read_bytes();identity=pathlib.Path(observed['state_dir'])/'vmm';identity.chmod(0);faults.append(identity)
  refused=start(BX,[unknown],expected='refused');assert refused['reason']=='disk-in-use' and U in refused['detail'] and '.owner' not in refused['detail']
  start(BX,[disk]);assert run(['box','exec',BX,'--','cat','/d0/marker']).stdout==b'CLAIM-CONTROL\n';down(BX)
  start(D,[second]);assert run(['box','exec',D,'--','printf','NEW-DISK']).stdout==b'NEW-DISK';down(D)
  for args in (['disk','list','--json'],['doctor']):
   report=run(args);assert report.returncode==0
  start(C);other=dead(C);down(C);assert not owner(C).exists() and not pathlib.Path(other['state_dir']).exists()
  assert owner(U).read_bytes()==saved and identity.exists();emit('readable unknown liveness: named disk pinned; unrelated existing/new admission, reporting and verified-dead targeted recovery succeeded')
  identity.chmod(0o600);faults.remove(identity);down(U)
  start(BX,[unknown]);assert run(['box','exec',BX,'--','cat','/d0/marker']).stdout==b'CLAIM-CONTROL\n';down(BX)
  emit('identity readability repair permitted checked orphan retirement and writable recovery with original marker')
 else:emit('NOT-RUN readable VMM-identity fault: Linux-only identity mechanism; opaque-claim controls exercised on Apple')
 emit('PASS scoped claim protection and positive controls; no global clean/prune/image removal')
finally:
 for path in faults:
  if path.exists():path.chmod(0o600)
 if command is not None and command.poll() is None:command.kill();command.communicate(timeout=30)
 for name in (A,BX,N,U,C,D):
  try:down(name)
  except Exception as error:emit('cleanup-box-failed='+str(error))
 for d in (disk,fresh,unknown,second):
  try:run(['disk','rm',d,'--json'])
  except Exception as error:emit('cleanup-disk-failed='+str(error))
 emit('fixture='+str(W))
