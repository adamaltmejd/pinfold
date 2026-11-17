"""Real CLI fix-round observations. Uses only the two existing images."""
import hashlib,json,os,pathlib,selectors,shutil,subprocess,tempfile,time
B=pathlib.Path('target/debug/pinfold').resolve(); W=pathlib.Path(tempfile.mkdtemp(prefix='pf279f.',dir='/tmp')).resolve()
E=dict(os.environ,XDG_CONFIG_HOME=str(W/'config'),XDG_STATE_HOME=str(W/'state'),PINFOLD_NO_UPDATE_CHECK='1')
stamp=W/'state/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch()
G=dict(E,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_SYSTEM='/dev/null',GIT_CONFIG_NOSYSTEM='1')
base='localhost/pinfold/profile-default'; owners={}; disks=[]; seq=0

def say(s): print(str(s).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/Users/user').rstrip(),flush=True)
def run(args,e=E,**kw):
 p=subprocess.run([str(a) for a in args],env=e,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180,**kw);return p

def pf(*args,**kw):return run([B,*args],**kw)
def m(p,g='/project',kind='share'):return dict(host=str(p),guest=g,kind=kind)
def spec(ms,**kw):
 global seq;seq+=1
 s=dict(name=f'probe-share279-fix-{os.getpid()}-{seq}',image=base,mounts=ms,env={'HOME':'/tmp'},cpus=2,memory='1G');s.update(kw);return s

def start(s):
 p=subprocess.Popen([str(B),'box','up'],env=E,text=True,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);owners[s['name']]=p
 p.stdin.write(json.dumps(s)+'\n');p.stdin.flush()
 with selectors.DefaultSelector() as sel:
  sel.register(p.stdout,selectors.EVENT_READ);assert sel.select(180),'startup timeout';v=json.loads(p.stdout.readline())
 say('up '+s['name']+': '+json.dumps(v));assert v['event']=='ready',v;return s['name']

def stop(n):
 p=pf('box','down',n);assert p.returncode==0,p.stdout
 o=owners.pop(n);o.stdin.close();o.wait(timeout=90);say(f'down {n}: owner={o.returncode} {o.stdout.read().strip()}');assert o.returncode==0
 assert pf('box','stat',n).returncode==3

def ex(n,c,label):
 p=pf('box','exec',n,'--','/bin/sh','-c',c);say(f'{label}: rc={p.returncode} '+p.stdout.strip().replace('\n',' | '));return p

def refuse(label,s,e=E):
 p=pf('box','up',e=e,input=json.dumps(s));v=json.loads(p.stdout);say(label+': '+p.stdout.strip());assert p.returncode==1 and v['reason']=='spec',p.stdout

def git(*args):
 p=run(['git','-c','core.hooksPath=/dev/null','-c','user.name=user','-c','user.email=user@example.invalid','-C',repo,*args],e=G);assert p.returncode==0,p.stdout;return p.stdout.strip()

def snap():
 rows=[]
 for p in sorted([repo/'.git',*(repo/'.git').rglob('*')]):
  s=p.lstat();rows.append((str(p.relative_to(repo)),s.st_mode,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns,hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else '-'))
 return hashlib.sha256(repr(rows).encode()).hexdigest()

repo=W/'repo'; other=W/'other';repo.mkdir();other.mkdir();(repo/'file').write_text('initial\n');(other/'file').write_text('readonly\n')
try:
 git('init','-q','--initial-branch=main');git('add','file');git('commit','-qm','initial')
 if os.environ.get('PF279_SOCKET')=='1':
  var=W/'readonly-var';(var/'host-services').mkdir(parents=True);(var/'host-services/ssh-auth.sock').touch()
  n=start(spec([m(var,'/var','readonly')],egress={}))
  assert ex(n,'stat -c "%a %F" /var/host-services/ssh-auth.sock','readonly socket-ancestor shape').returncode==0;stop(n)
 else:
  if os.environ.get('PF279_TAIL')!='1':
   before=snap()
   for suffix in ['.GIT/hooks','.GIT/new-dir']:
    refuse('case destination '+suffix,spec([m(repo),m(other,'/project/'+suffix,'readonly')]))
   assert snap()==before;say('case refusals: protected bytes and metadata unchanged=True')
   for dest in ['/project/*','/project/*/','/work/../*']:
    refuse('Git pattern '+dest,spec([m(repo,dest)]))
   # Externally linked artifact and selected-profile directories; no cache is prepared.
   cache=W/'cache';(cache/'pinfold').mkdir(parents=True);external=W/'external-artifacts';(external/'init/subdir').mkdir(parents=True)
   (cache/'pinfold/artifacts').symlink_to(external);ce=dict(E,XDG_CACHE_HOME=str(cache))
   refuse('external artifacts descendant',spec([m(external/'init/subdir')]),ce)
   prof=W/'external-profile';(prof/'share/work').mkdir(parents=True);(prof/'work').mkdir()
   profiles=W/'config/pinfold/profiles';profiles.mkdir(parents=True);(profiles/'probe-profile').symlink_to(prof)
   refuse('external profile directory descendant',spec([m(prof/'work')],profile='probe-profile'))
   plain=profiles/'probe-share';plain.mkdir();(plain/'share').symlink_to(prof/'share')
   refuse('external profile share descendant',spec([m(prof/'share/work')],profile='probe-share'))
   n=start(spec([m(repo,'/work/star*name')]))
   assert ex(n,"git -C '/work/star*name' rev-parse --show-toplevel",'literal embedded star Git read').returncode==0
   data=json.loads(run(['container','inspect',n]).stdout)[0]
   initdir=next(x['destination'] for x in data['configuration']['mounts'] if '/artifacts/init/' in x['destination'])
   stop(n)
   # These shapes were valid before share admission was introduced.
   ro=W/'ro';(ro/'nested/child').mkdir(parents=True);(ro/'link').symlink_to('nested')
   n=start(spec([m(ro,'/parent','readonly'),m(other,'/parent/link/child','readonly')]))
   assert ex(n,'cat /parent/link/child/file','readonly nested symlink shape').returncode==0;stop(n)
   ro_users=W/'users';(ro_users/pathlib.Path(initdir).relative_to('/Users')).mkdir(parents=True)
   n=start(spec([m(ro_users,'/Users','readonly')]))
   assert ex(n,'id -u','readonly init-ancestor shape').returncode==0;stop(n)
   disk='probe-share279-fix-init-disk';disks.append(disk)
   n=start(spec([],disks=[dict(name=disk,guest='/Users',size='1G')]))
   assert ex(n,'printf disk > /Users/fixture; cat /Users/fixture','disk init-ancestor shape').returncode==0;stop(n)
   disk='probe-share279-fix-from-git';disks.append(disk)
   n=start(spec([],disks=[dict(name=disk,guest='/work',size='1G',**{'from':str(repo/'.git')})]))
   assert ex(n,'test -f /work/HEAD && echo copied-Git-data','disk from Git is a copy').returncode==0;stop(n)
   outer=W/'outer';(outer/'child').mkdir(parents=True);(outer/'child/file').write_text('overlap')
   n=start(spec([m(outer,'/one'),m(outer/'child','/two')]))
   assert ex(n,'printf allowed > /two/file','overlapping no-Git shares').returncode==0
   assert (outer/'child/file').read_text()=='allowed';stop(n)
   (repo/'child').mkdir()
   n=start(spec([m(repo,'/one'),m(repo/'child','/two')]))
   assert ex(n,'printf allowed > /two/file','overlap outside protected Git').returncode==0;stop(n)
   n=start(spec([m(repo,'/one'),m(repo,'/two')]))
   assert ex(n,'git -C /one rev-parse HEAD; git -C /two rev-parse HEAD','same-root shares protect both views').returncode==0;stop(n)
   refuse('overlap exposes nested protected Git',spec([m(W,'/ancestor'),m(repo,'/nested')]))
   # Use a fixtures-only ancestor without protected XDG roots for the exposure predicate.
   tree=W/'tree';(tree/'nested/.git').mkdir(parents=True)
   refuse('ancestor exposes nested protected Git',spec([m(tree,'/ancestor'),m(tree/'nested','/nested')]))
  (repo/'bin').mkdir();chmod=repo/'bin/chmod';chmod.write_text('#!/bin/sh\n/usr/bin/id -u > /project/old-root-marker\nexit 0\n');chmod.chmod(0o755)
  n=start(spec([m(repo)],image='localhost/pinfold/image-probe-share279-env:latest',egress={}))
  assert ex(n,'test ! -e /project/old-root-marker; stat -c "%a %F" /var/host-services/ssh-auth.sock','fixed root operation').returncode==0
  assert not (repo/'old-root-marker').exists()
  assert ex(n,'printf harmless > /tmp/harmless','positive-control harmless target').returncode==0
  p=run(['container','exec','--user','0:0',n,'chmod','666','/tmp/harmless']);say(f'old root PATH positive control: rc={p.returncode} marker='+(repo/'old-root-marker').read_text().strip());assert p.returncode==0 and (repo/'old-root-marker').read_text().strip()=='0'
  protected=snap()
  for c in ['mv /project/.GIT /project/moved','rmdir /project/.GIT','printf x >> /project/.GIT/HEAD']:
   assert ex(n,c,'case protected-name operation').returncode!=0 and snap()==protected
  assert ex(n,'git -C /project status --short >/dev/null && git -C /project log -1 --format=%H','before host commit Git').returncode==0
  live=subprocess.Popen([str(B),'box','exec',n,'--','/bin/sh','-c','echo LIVE; count=0; errors=0; while [ ! -e /project/gate ]; do git -C /project log -1 --format=%H >/dev/null 2>&1 || errors=$((errors+1)); printf live > /project/live; count=$((count+1)); done; echo reads=$count errors=$errors'],env=E,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  assert live.stdout.readline().strip()=='LIVE'
  (repo/'file').write_text('host commit\n');git('add','file');git('commit','-qm','host-commit-fix-round');expected=git('rev-parse','HEAD');t=time.monotonic();protected=snap();(repo/'gate').touch();out=live.communicate(timeout=90)[0];say('concurrent Git: '+out.strip());assert live.returncode==0
  recovered=False;attempt=0
  while time.monotonic()-t<20:
   p=pf('box','exec',n,'--','/bin/sh','-c','git -C /project status --short; status=$?; git -C /project log -1 --format=%H; log=$?; echo status=$status log=$log; test "$status:$log" = 0:0')
   attempt+=1
   if attempt==1:say('immediate Git: '+p.stdout.strip().replace('\n',' | '))
   if p.returncode==0 and expected in p.stdout:recovered=True;break
  say(f'later Git checkpoint: recovered={recovered} observations={attempt} elapsed={time.monotonic()-t:.3f}s '+p.stdout.strip().replace('\n',' | '))
  say('host commit protected snapshot unchanged='+str(snap()==protected));assert snap()==protected;stop(n)

finally:
 for n in list(owners):stop(n)
 for disk in disks:
  p=pf('disk','rm',disk,'--json');say('disk cleanup: '+p.stdout.strip());assert p.returncode==0
 shutil.rmtree(W);say('fix-round fixtures removed='+str(not W.exists()))
