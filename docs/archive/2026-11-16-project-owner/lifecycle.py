#!/usr/bin/env python3
"""Real lifecycle acceptance: lifecycle.py BIN IMAGE; no timing retries."""
import fcntl,hashlib,json,os,pathlib,select,signal,subprocess as sp,sys,tempfile
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfol.',dir='/tmp')).resolve();A=W/('s'*60)/('t'*60)
S=A/('u'*(220-len(str(A))-1));assert len(os.fsencode(S))==220
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(S),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(S/'pinfold').mkdir(parents=True);(S/'pinfold/maintenance').touch()
H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}';P=W/'p';Q=W/'q';foreign=[];children=[]
def emit(s):print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def git(p,*args):return sp.run(['git','-C',str(p),*args],check=True,capture_output=True).stdout.strip()
def run(args,code=0,cwd=P,data=b''):
 r=sp.run([B]+args,cwd=cwd,env=E,input=data,capture_output=True,timeout=180)
 emit(f'{args!r} exit={r.returncode} out={r.stdout[:150]!r} err={r.stderr[:350]!r}');assert r.returncode==code,'CLI status mismatch';return r
for p in (P,Q):
 p.mkdir();git(p,'init','-q');(p/'base').write_text('BASE\n');git(p,'add','base')
 git(p,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','fixture')
 run(['config','edit'],cwd=p);ident=p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
 c=W/'c/pinfold/projects'/ident/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
def ident(p):return p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
def locator(p=P):return H/(hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()+'.project')
def loc(p=P):return json.loads(locator(p).read_text())
def record(l):return H/(hashlib.sha256(('box:'+l['name']).encode()).hexdigest()+'.owner')
def sockets(l):return H/('r-'+l['generation'][:32])
def terminal(l):return S/'pinfold/runs'/l['generation']/'terminal.json'
def process(args,cwd=P):
 p=sp.Popen([B]+args,cwd=cwd,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);children.append(p);return p
def line(p):
 assert select.select([p.stdout],[],[],180)[0],'guest marker not received'
 got=p.stdout.readline();assert got,'guest ended before marker';return got
def complete(p,code):
 out,err=p.communicate(timeout=180);emit(f'client exit={p.returncode} out={out[:120]!r} err={err[:300]!r}');assert p.returncode==code,'client status mismatch';return out

def lock_exclusive(held):
 with open(B,'rb') as f:
  try:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
  except BlockingIOError:assert held,'executable still owned';return
  assert not held,'kept owner lost executable ownership'

def readonly(positive):
 name=f'probe-owner-ro-{os.getpid()}';disk='pi-'+ident(P)+'-project'
 spec={'name':name,'image':IMAGE,'cpus':2,'memory':'1G','disks':[{'name':disk,'guest':'/data','readonly':True}]}
 data=json.dumps(spec).encode()+b'\n'
 if not positive:
  r=run(['box','up'],1,data=data);assert json.loads(r.stdout)['reason']=='disk-unclean';return
 p=process(['box','up']);p.stdin.write(data);p.stdin.flush();ready=json.loads(line(p));assert ready['event']=='ready'
 assert not locator().exists(),'raw core up created a project lease'
 assert run(['box','exec',name,'--','cat','/data/durable']).stdout==b'DURABLE\n'
 p.stdin.close();p.stdin=None;complete(p,0)

def kill_owner(killed):
  listed=[json.loads(x) for x in run(['box','list','--label','dev.pinfold.owner']).stdout.splitlines()]
  owner=next(x['owner'] for x in listed if x['name']==killed['name'] and x['labels']['dev.pinfold.generation']==killed['generation'])
  handles=[]
  if sys.platform=='linux':
   handles.append(os.pidfd_open(owner));r=json.loads(record(killed).read_text());vmm=int((pathlib.Path(r['state_dir'])/'vmm').read_text().split()[0]);handles.append(os.pidfd_open(vmm))
   signal.pidfd_send_signal(handles[0],signal.SIGKILL)
  else:os.kill(owner,signal.SIGKILL)
  for fd in handles:assert select.select([fd],[],[],30)[0],'owner/VMM survived';os.close(fd)

try:
 run(['start'],cwd=Q);witness=loc(Q);witness_inode=sockets(witness).joinpath('c.sock').stat().st_ino
 def witness_ok():
  assert run(['--','sh','-c','printf WITNESS'],cwd=Q).stdout==b'WITNESS'
  assert loc(Q)==witness and sockets(witness).joinpath('c.sock').stat().st_ino==witness_inode
 run(['start']);old=loc();old_head=git(P,'rev-parse','HEAD');branch=git(P,'symbolic-ref','--short','HEAD').decode()
 # Kept owners with no clients retain maintenance and executable ownership.
 before=(S/'pinfold/projects'/ident(P)/'state.json').read_bytes();lock_exclusive(True)
 run(['clean','--dry-run','--unused','0s']);assert (S/'pinfold/projects'/ident(P)/'state.json').read_bytes()==before
 refused=run(['disk','rm','pi-'+ident(P)+'-project'],1);assert b'disk-in-use' in refused.stderr
 emit('kept no-client: state bytes preserved by dry-run; scoped disk removal refused; exclusive executable lock denied')
 # Joining excludes host commits and performs no new-owner fast-forward.
 (P/'host-change').write_text('HOST\n');git(P,'add','host-change');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','host-change')
 new_head=git(P,'rev-parse','HEAD');assert new_head!=old_head
 assert run(['--','git','rev-parse','HEAD']).stdout.strip()==old_head and loc()==old
 assert run(['--','sh','-c','test ! -e host-change']).returncode==0
 run(['push']);assert run(['--','git','rev-parse','refs/remotes/host/'+branch]).stdout.strip()==new_head
 assert run(['--','git','rev-parse','HEAD']).stdout.strip()==old_head
 run(['--','git','-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','guest'])
 guest_head=run(['--','git','rev-parse','HEAD']).stdout.strip();run(['fetch'])
 assert git(P,'rev-parse','refs/remotes/pinfold/'+branch)==guest_head
 run(['stop']);run(['push']);run(['fetch']);assert not locator().exists()
 emit('join left checkout unchanged; explicit Borrow push/fetch and stopped-project transient push/fetch passed')
 # Stdin EOF only ends input. A second lease releases the guest FIFO barrier.
 p=process(['--','sh','-c','mkfifo /tmp/probe-owner-eof; cat >/dev/null; echo EOF-OBSERVED; read x </tmp/probe-owner-eof; echo EOF-CONTINUED'])
 p.stdin.close();p.stdin=None;assert line(p)==b'EOF-OBSERVED\n';eof_owner=loc()
 assert run(['--','sh','-c','echo release >/tmp/probe-owner-eof']).returncode==0
 assert complete(p,0)==b'EOF-CONTINUED\n';run(['stop'])
 emit('stdin EOF retained a running lease until FIFO release')
 # A final client crash ends the unkept owner. Name flock is the barrier.
 p=process(['--','sh','-c','echo LAST-CLIENT; read x']);assert line(p)==b'LAST-CLIENT\n';last=loc()
 p.kill();complete(p,-signal.SIGKILL)
 with open(H/(hashlib.sha256(('box:'+last['name']).encode()).hexdigest()+'.lock'),'rb') as f:fcntl.flock(f,fcntl.LOCK_EX)
 assert not locator().exists() and not record(last).exists()
 t=json.loads(terminal(last).read_text());assert t['reason']=='sessions-ended' and t['detail'] is None
 emit('last-client SIGKILL completed teardown and sessions-ended/null; witness socket unchanged');witness_ok()
 p=process(['--','sh','-c','echo OWNER-ACTIVE; read x']);assert line(p)==b'OWNER-ACTIVE\n';active=loc();kill_owner(active);p.wait(timeout=100);complete(p,125);assert not terminal(active).exists();witness_ok();run(['stop']);assert run(['--','sh','-c','printf ACTIVE-RECOVERED']).stdout==b'ACTIVE-RECOVERED'
 emit('active owner SIGKILL: incomplete client=125; witness usable; no inferred terminal; checked restart=ACTIVE-RECOVERED')
 # Crash a kept owner, retire its orphan without cleaning dirty disks, then recover RW.
 run(['start']);killed=loc();run(['--','sh','-c','printf "DURABLE\\n" > durable; sync'])
 kill_owner(killed)
 assert not terminal(killed).exists();witness_ok();run(['stop']);assert not terminal(killed).exists()
 readonly(False);assert run(['--','cat','durable']).stdout==b'DURABLE\n';readonly(True)
 assert not terminal(killed).exists();emit('kept owner crash: no terminal fabricated; read-only dirty refusal followed by RW recovery and read-only control')
 # Unexpected entry makes real socket cleanup fail after observed command completion.
 for code,expected in ((0,125),(23,23)):
  p=process(['--','sh','-c',f'echo CLEANUP-BARRIER; read x; exit {code}']);assert line(p)==b'CLEANUP-BARRIER\n';broken=loc()
  bad=sockets(broken)/'probe-foreign';bad.write_bytes(b'UNCHANGED\n');bad.chmod(0o600);foreign.append(bad)
  p.stdin.write(b'release\n');p.stdin.flush();complete(p,expected);assert bad.read_bytes()==b'UNCHANGED\n'
  bad.unlink();foreign.remove(bad);run(['stop']);assert run(['--','sh','-c','printf RESTART']).stdout==b'RESTART';witness_ok()
  emit(f'actual cleanup failure: observed={code} result={expected}; foreign bytes preserved; targeted recovery and restart passed')
 run(['stop']);run(['stop'],cwd=Q);lock_exclusive(False)
 emit('exclusive executable lock available after both kept owners stopped')
 emit('SKIP verified update replacement/update-busy CLI: requires isolated HTTPS release fixture; kernel executable-lock boundary exercised only')
 emit('SKIP real clean --unused: CLI may delete shared unowned image/cache resources; dry-run and scoped disk-removal controls exercised')
 emit('PASS long-state bytes=220; lifecycle, dirty-disk recovery, crossing and cleanup-status cases')
finally:
 for bad in foreign:
  if bad.exists() and bad.read_bytes()==b'UNCHANGED\n':bad.unlink()
 for p in children:
  if p.poll() is None:p.kill();p.communicate(timeout=30)
 for p in (P,Q):
  try:run(['stop'],cwd=p)
  except Exception as error:emit('cleanup-stop-failed='+str(error))
 for p in (P,Q):
  for suffix in ('home','project'):
   try:run(['disk','rm','pi-'+ident(p)+'-'+suffix])
   except Exception as error:emit('cleanup-disk-failed='+str(error))
 emit('fixture='+str(W))
