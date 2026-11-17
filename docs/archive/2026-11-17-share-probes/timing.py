import hashlib,json,os,pathlib,shutil,stat,subprocess as sp,tempfile,time,uuid
D=pathlib.Path(__file__).resolve().parent;ROOT=D.parents[2];I='localhost/pinfold/profile-default:latest'
E={**os.environ,'GIT_CONFIG_GLOBAL':'/dev/null','GIT_CONFIG_NOSYSTEM':'1','GIT_AUTHOR_NAME':'user','GIT_AUTHOR_EMAIL':'user@fixture.invalid','GIT_COMMITTER_NAME':'user','GIT_COMMITTER_EMAIL':'user@fixture.invalid'}
def emit(x):print(json.dumps(x),flush=True)
def walk(root,skip_git=False):
 with os.scandir(root) as it:
  for e in it:
   if skip_git and e.name=='.git':continue
   s=e.stat(follow_symlinks=False);yield pathlib.Path(e.path),s
   if stat.S_ISDIR(s.st_mode):yield from walk(e.path)
def admit(root):
 start=time.perf_counter();protected=set();symlinks=0;files=0
 if (root/'.git').is_dir():
  for p,s in walk(root/'.git'):
   symlinks+=stat.S_ISLNK(s.st_mode)
   if stat.S_ISREG(s.st_mode) and s.st_nlink>1:protected.add((s.st_dev,s.st_ino))
 aliases=[]
 for p,s in walk(root,True):
  files+=1
  if stat.S_ISREG(s.st_mode) and (s.st_dev,s.st_ino) in protected:aliases.append(str(p.relative_to(root)))
 return dict(seconds=round(time.perf_counter()-start,6),entries=files,protected_linked=len(protected),symlinks=symlinks,aliases=aliases)
def git(r,*a):return sp.run(['git','-C',str(r),*a],env=E,capture_output=True,check=True).stdout
W=pathlib.Path(tempfile.mkdtemp(prefix='pft.',dir=os.environ['TMPDIR'])).resolve();R=W/'repo';R.mkdir();names=[]
def snapshot():return {str(p.relative_to(R)):hashlib.sha256(p.read_bytes()).hexdigest() for p in (R/'.git').rglob('*') if p.is_file()}
def call(a):return sp.run(a,env=E,text=True,capture_output=True,timeout=300)
try:
 emit({'host_git':call(['git','--version']).stdout.strip(),'uid':os.getuid(),'fixture':'100001 tracked files, 1000 untracked files in 10 directories; one tracked edit'})
 git(R,'init','-q','-b','main');(R/'a').write_text('a\n')
 for i in range(100000):
  p=R/f'd{i//1000:03}'/f'f{i:06}';p.parent.mkdir(exist_ok=True);p.write_text('x\n')
 git(R,'add','.');git(R,'commit','-qm','fixture')
 for i in range(1000):
  p=R/f'u{i//100:03}'/f'f{i:06}';p.parent.mkdir(exist_ok=True);p.write_text('u\n')
 (R/'a').write_text('edited\n')
 for label,r in [('100001',R),('built-repository',ROOT)]:
  for i in range(3):emit({'admission':label,'sample':i,**admit(r)})
 # Separate tiny fixtures exercise admission, including an allowed outside hardlink.
 T=W/'tiny';T.mkdir();(T/'.git').mkdir();(T/'.git/p').write_text('p');(W/'outside').write_text('o');os.link(W/'outside',T/'outside-alias')
 emit({'admission_case':'outside-hardlink',**admit(T)});os.link(T/'.git/p',T/'protected-alias');emit({'admission_case':'protected-hardlink',**admit(T)});(T/'protected-alias').unlink();(T/'.git/link').symlink_to('../outside-alias');emit({'admission_case':'git-symlink',**admit(T)})
 commands=[['--no-optional-locks','status','--short','--ignore-submodules=dirty'],['status','--porcelain','--ignore-submodules=dirty']]
 expected=' M a\n'+''.join(f'?? u{i:03}/\n' for i in range(10))
 js='my $t=clock_gettime(CLOCK_MONOTONIC); open my $f,"-|","git",@ARGV or die $!; local $/; my $o=<$f>;close $f; print encode_json({git_seconds=>clock_gettime(CLOCK_MONOTONIC)-$t,rc=>$?>>8,out=>$o,err=>""})';
 for cache in [False,True]:
  E.update(GIT_CONFIG_COUNT='1',GIT_CONFIG_KEY_0='safe.directory',GIT_CONFIG_VALUE_0=str(R));git(R,'config','core.untrackedCache',str(cache).lower());git(R,'update-index','--untracked-cache' if cache else '--no-untracked-cache')
  if cache:git(R,'status','--porcelain')
  before=snapshot();n='probe-status-'+uuid.uuid4().hex[:6];names.append(n)
  p=call(['container','run','-d','--name',n,'--network','none','--read-only','--cap-drop','ALL','--user',f'{os.getuid()}:{os.getgid()}','--cpus','4','--memory','4G','-e','GIT_CONFIG_GLOBAL=/dev/null','-e','GIT_CONFIG_NOSYSTEM=1','-e','GIT_CONFIG_COUNT=1','-e','GIT_CONFIG_KEY_0=safe.directory','-e',f'GIT_CONFIG_VALUE_0={R}','-v',f'{R}:{R}','-v',f'{R}/.git:{R}/.git:ro','--entrypoint','sleep',I,'infinity']);assert p.returncode==0,p.stderr
  emit({'cache':cache,'guest_git':call(['container','exec',n,'git','--version']).stdout.strip(),'cold_definition':'new VM for each cache condition; first status only; no OS host cache flush'})
  for i in range(3):
   t=time.perf_counter();p=call(['container','exec',n,'true']);emit({'cache':cache,'exec_only':i,'seconds':round(time.perf_counter()-t,6),'rc':p.returncode})
  for ci,args in enumerate(commands):
   emit({'cache':cache,'command':ci,'argv':['git',*args]})
   for i in range(3):
    t=time.perf_counter();p=call(['git','-C',str(R),*args]);emit({'cache':cache,'command':ci,'sample':i,'side':'host','git_seconds':round(time.perf_counter()-t,6),'rc':p.returncode,'correct':p.stdout==expected})
    before_guest=snapshot();t=time.perf_counter();p=call(['container','exec','-w',str(R),n,'perl','-MTime::HiRes=clock_gettime,CLOCK_MONOTONIC','-MJSON::PP','-e',js,'--',*args]);outer=time.perf_counter()-t;assert p.returncode==0,(p.stdout,p.stderr);row=json.loads(p.stdout);emit({'cache':cache,'command':ci,'sample':i,'side':'guest','git_seconds':round(row['git_seconds'],6),'exec_total':round(outer,6),'rc':row['rc'],'correct':row['out']==expected,'stderr':p.stderr,'protected_files_unchanged':snapshot()==before_guest})
  emit({'cache':cache,'protected_files_unchanged_across_host_and_guest':snapshot()==before})
  call(['container','stop',n]);call(['container','delete',n]);names.remove(n)
finally:
 for n in names:call(['container','stop',n]);call(['container','delete',n])
 shutil.rmtree(W);emit({'fixture_and_own_containers_removed':True})
