#!/usr/bin/env python3
"""Real existing-disk admission and per-claim measurement controls; no sleeps."""
import hashlib,json,os,select,subprocess as sp,sys,tempfile
from pathlib import Path
binary,image=str(Path(sys.argv[1]).resolve()),sys.argv[2]
before=str(Path(sys.argv[3]).resolve()) if len(sys.argv)>3 else None
linux=sys.platform.startswith('linux');assert os.geteuid()!=0
work=Path(tempfile.mkdtemp(prefix='pfrr5.',dir='/tmp')).resolve()
env={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(work/'c'),XDG_STATE_HOME=str(work/'s'),PINFOLD_NO_UPDATE_CHECK='1')
(work/'s/pinfold').mkdir(parents=True);(work/'s/pinfold/maintenance').touch()
host=Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
prefix=f'probe-rereview5-{os.getpid()}'
a,b,n,u,c,e=[prefix+'-'+x for x in ('a','b','new','unsafe','dead','corrupt')]
disk,fresh=prefix+'-filled',prefix+'-fresh'
children={};bad_record=unsafe_lock=None;original=None
seed=work/'seed';seed.mkdir();(seed/'marker').write_bytes(b'DISK-CONTROL\n')
def emit(text):
 print(text.replace(binary,'$BIN').replace(str(work),'$W').replace(str(Path.home()),'/home/user'),flush=True)
def owner(name):return host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
def lock(name):return owner(name).with_suffix('.lock')
def run(args,code=0,tool=binary):
 r=sp.run([tool]+args,env=env,cwd=work,input=b'',capture_output=True,timeout=180)
 emit(f'$ {"$BEFORE" if tool!=binary else "$BIN"} {args!r}; exit={r.returncode} stdout={r.stdout!r} stderr={r.stderr!r}')
 if code is not None:assert r.returncode==code,r
 return r
def start(name,attached=(),expected='ready'):
 spec={'name':name,'image':image,'cpus':2,'memory':'1G','disks':[{'name':d,'guest':f'/d{i}','size':'1G',**({'from':str(seed)} if d==disk else {})} for i,d in enumerate(attached)]}
 p=sp.Popen([binary,'box','up'],cwd=work,env=env,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
 p.stdin.write(json.dumps(spec).encode()+b'\n');p.stdin.flush()
 poll=select.poll();poll.register(p.stdout,select.POLLIN|select.POLLHUP);assert poll.poll(180000),'up readiness deadline'
 event=json.loads(p.stdout.readline());emit(f'up {name}: {event!r}')
 assert event['event']==expected,event
 if expected=='ready':children[name]=p
 else:
  out,err=p.communicate(timeout=180);emit(f'refused/failed owner exit={p.returncode} stderr={err!r}');assert p.returncode==1
 return event
def down(name):
 run(['box','down',name])
 if name in children:
  p=children.pop(name);out,err=p.communicate(timeout=180);assert p.returncode==0,(out,err,p.returncode)
def snapshot():
 path=work/'s/pinfold/disks'/(disk+'.ext4');meta=path.stat();h=hashlib.sha256()
 with path.open('rb') as f:
  for block in iter(lambda:f.read(1<<20),b''):h.update(block)
 return meta.st_ino,meta.st_size,h.hexdigest()
def dead(name):
 record=json.loads(owner(name).read_bytes());pid=int((Path(record['state_dir'])/'vmm').read_text().split()[0])
 fd=os.pidfd_open(pid);p=children.pop(name);p.kill();p.wait(timeout=30)
 poll=select.poll();poll.register(fd,select.POLLIN)
 try:assert poll.poll(30000),'VMM death deadline'
 finally:os.close(fd)
 for stream in (p.stdin,p.stdout,p.stderr):stream.close()
 assert p.returncode==-9;emit(f'owner {name} SIGKILL=-9; actual VMM pidfd readable')
 return record
try:
 start(a,[disk])
 command=sp.Popen([binary,'box','exec',a,'--','sh','-c',"cat /d0/marker; sync; printf 'ACTIVE\\n'; read answer; [ \"$answer\" = continue ]; cat /d0/marker; printf 'STILL-ACTIVE\\n'"],cwd=work,env=env,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
 assert command.stdout.readline()==b'DISK-CONTROL\n';assert command.stdout.readline()==b'ACTIVE\n'
 assert command.poll() is None
 initial=snapshot() if linux else None
 bad_record=owner(a);bad_record.chmod(0)
 refused=start(b,[disk],expected='refused');assert refused['reason']=='disk-in-use' and bad_record.name in refused['detail']
 if linux:
  assert snapshot()==initial;emit(f'A full disk backing inode/size/SHA256 unchanged across B refusal: {initial!r}')
 out,err=command.communicate(b'continue\n',timeout=30)
 assert command.returncode==0 and out==b'DISK-CONTROL\nSTILL-ACTIVE\n' and not err
 assert children[a].poll() is None;emit('A pre-existing guest command continued while owner record unreadable: exit=0 DISK-CONTROL/STILL-ACTIVE')
 start(n,[fresh]);assert run(['box','exec',n,'--','printf','NEW-ONLY-CONTROL']).stdout==b'NEW-ONLY-CONTROL';down(n)
 listed=run(['disk','list','--json']);assert {json.loads(line)['name'] for line in listed.stdout.splitlines()}=={disk,fresh}
 bad_record.chmod(0o600);bad_record=None
 refused=start(b,[disk],expected='refused');assert refused['reason']=='disk-in-use' and a in refused['detail'] and '.owner' not in refused['detail']
 emit('record repair cleared the unreadable-record refusal; live A still owns its named writable disk')
 down(a);start(b,[disk]);assert run(['box','exec',b,'--','cat','/d0/marker']).stdout==b'DISK-CONTROL\n';down(b)
 emit('same B request succeeds after normal A down, preserving the seeded bytes')
 run(['disk','rm',disk,'--json'])
 if linux:
  start(e);saved=owner(e).read_bytes();dead(e);owner(e).write_bytes(b'{');original=(owner(e),saved)
  run(['box','down',e],code=1);run(['box','prune']);start(e,expected='failed')
  assert owner(e).read_bytes()==b'{';emit('dead owner with corrupt JSON: down/up failed and prune preserved it; no CLI repaired the record')
  owner(e).write_bytes(saved);original=None;down(e)
  emit('restoring known-valid original JSON allowed existing box down to recover the name')
  start(u);observed=dead(u);u_record=owner(u).read_bytes();state=Path(observed['state_dir']);identity=(state/'vmm').read_bytes()
  generation=host/('r-'+observed['generation'][:32]);marker=(generation/'generation').read_bytes()
  unsafe_lock=lock(u);unsafe_lock.chmod(0o644)
  start(c);other=dead(c)
  if before:
   old=run(['clean','--dry-run'],code=1,tool=before);assert b'ownership lock' in old.stderr
   old=run(['doctor'],tool=before);assert b'  disks:' not in old.stdout
   emit('before-fix binary: unsafe unrelated name lock aborted dry-run and suppressed doctor disk sizes')
  for args in (['doctor'],['clean','--dry-run']):
   measured=run(args);assert b'  disks:' in measured.stdout and u.encode() in measured.stderr and b'socket measurement' in measured.stderr
  assert {p for p in host.glob('*.owner')}=={owner(u),owner(c)},'foreign claims prevent actual clean'
  cleaned=run(['clean']);assert b'  disks:' in cleaned.stdout and u.encode() in cleaned.stderr
  assert not owner(c).exists() and not Path(other['state_dir']).exists()
  assert owner(u).read_bytes()==u_record and state.exists() and (state/'vmm').read_bytes()==identity and (generation/'generation').read_bytes()==marker
  emit('actual clean removed verified-dead C; unsafe-lock U record/state/identity/generation preserved; warnings and disk sizes retained')
  unsafe_lock.chmod(0o600);unsafe_lock=None;down(u)
  start(u);assert run(['box','exec',u,'--','printf','LOCK-CONTROL']).stdout==b'LOCK-CONTROL';down(u)
  emit('correcting only name lock allowed targeted down and same-name start/exec/down')
 run(['disk','rm',fresh,'--json'])
 emit('PASS existing disk admission and controls'+('; per-claim measurement/recovery controls' if linux else ' on Apple'))
finally:
 if bad_record is not None and bad_record.exists():bad_record.chmod(0o600)
 if unsafe_lock is not None and unsafe_lock.exists():unsafe_lock.chmod(0o600)
 if original is not None and original[0].exists():original[0].write_bytes(original[1])
 for name in (a,b,n,u,c,e):
  run(['box','down',name],code=None)
  if name in children:
   p=children[name]
   if p.poll() is None:p.communicate(timeout=180)
 for d in (disk,fresh):run(['disk','rm',d,'--json'],code=None)
