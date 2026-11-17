"""Public CLI acceptance. Existing image only; private disposable projects."""
import hashlib,json,os,pathlib,select,shlex,shutil,signal,subprocess as sp,tempfile,time
B=pathlib.Path('target/debug/pinfold').resolve()
W=pathlib.Path(tempfile.mkdtemp(prefix='pf280.',dir='/tmp')).resolve()
E={k:os.environ[k] for k in ('PATH','HOME','TMPDIR') if k in os.environ}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',VISUAL='/usr/bin/true',EDITOR='/usr/bin/true',GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
stamp=W/'s/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch()
projects=[];clients=[]
def say(s):print(str(s).replace(str(W),'$W').replace(str(B),'$BIN').replace(str(pathlib.Path.home()),'/Users/user'),flush=True)
def raw(a,cwd=None,e=E,**kw):return sp.run(list(map(str,a)),cwd=cwd,env=e,capture_output=True,timeout=180,**kw)
def pf(p,*a,code=0,e=E):
 r=raw([B,*a],p,e,input=b'');say(f'CLI {list(a)!r} cwd={p}: rc={r.returncode} out={r.stdout.decode(errors="replace").strip()[:1000]!r} err={r.stderr.decode(errors="replace").strip()[:500]!r}')
 if code is not None:assert r.returncode==code,(a,r.returncode,r.stdout,r.stderr)
 return r

def git(p,*a):
 r=raw(['git','-c','core.hooksPath=/dev/null','-c','user.name=user','-c','user.email=user@fixture.invalid','-C',p,*a]);assert r.returncode==0,r.stderr;return r.stdout.decode().strip()
def config(p):return W/'c/pinfold/projects'/(p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12])/'pinfold.toml'
def setup(name,repo=True,kind='share'):
 p=W/name;p.mkdir();projects.append(p);(p/'file').write_text('initial\n')
 if repo:git(p,'init','-q','-b','main');git(p,'add','file');git(p,'commit','-qm','initial')
 pf(p,'config','edit');c=config(p);assert 'kind = "share"' in c.read_text()
 c.write_text(f'mounts = [{{ host = ".", kind = "{kind}" }}]\nimage = "localhost/pinfold/profile-default"\nallow = []\n')
 return p

def spawn(p,script):
 say(f'SPAWN pinfold -- sh -c {script!r} cwd={p}')
 c=sp.Popen([str(B),'--','sh','-c',script],cwd=p,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);clients.append(c);return c

def line(c):
 assert select.select([c.stdout],[],[],120)[0],'command readiness timeout'
 s=c.stdout.readline().decode().strip();assert s,(c.poll(),c.stderr.read().decode());say('READY '+s);return s

def done(c,code=0):
 out,err=c.communicate(timeout=180);say(f'CLIENT rc={c.returncode} out={out.decode(errors="replace").strip()!r} err={err.decode(errors="replace").strip()[:500]!r}');assert c.returncode==code

def listed(p):
 ident=p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
 r=pf(p,'box','list','--label','dev.pinfold.project='+ident)
 return [json.loads(x) for x in r.stdout.splitlines()]
def snap(p):
 rows=[]
 for x in sorted([p/'.git',*(p/'.git').rglob('*')]):
  s=x.lstat();rows.append((str(x.relative_to(p)),s.st_mode,s.st_dev,s.st_ino,s.st_nlink,s.st_size,s.st_uid,s.st_gid,s.st_mtime_ns,s.st_ctime_ns,hashlib.sha256(x.read_bytes()).hexdigest() if x.is_file() else '-'))
 return hashlib.sha256(repr(rows).encode()).hexdigest()
def wait_file(p):
 end=time.monotonic()+20
 while not p.exists():
  assert time.monotonic()<end,'missing fixture readiness'
  select.select([],[],[],.01)
def ex(p,script,code=0):return pf(p,'--','sh','-c',script,code=code)
try:
 p=setup('race');before=snap(p)
 # Simultaneous first sessions, shared file writes, departing client cancellation.
 a=spawn(p,'echo one >> shared; echo FIRST; while read x; do echo "$x" >> shared; echo WROTE; done')
 b=spawn(p,'echo two >> shared; echo SECOND; while read x; do echo "$x" >> shared; echo WROTE; done')
 line(a);line(b);boxes=listed(p);assert len(boxes)==1;generation=boxes[0]['labels']['dev.pinfold.generation'];say('one-generation='+generation)
 a.stdin.write(b'alpha\n');a.stdin.flush();line(a);b.stdin.write(b'beta\n');b.stdin.flush();line(b)
 a.terminate();done(a,143);b.stdin.write(b'after-one-left\n');b.stdin.flush();line(b)
 assert snap(p)==before;say('two-session edits='+repr((p/'shared').read_text())+' protected-bytes-and-metadata-unchanged=True')
 pf(p,'start');b.stdin.close();b.stdin=None;done(b);assert listed(p)[0]['labels']['dev.pinfold.generation']==generation
 # Keep/departure racing join, then stop racing an active session.
 c=spawn(p,'echo JOIN; cat >/dev/null');line(c)
 pf(p,'stop');done(c,125);assert not listed(p);pf(p,'stop')
 # Final unkept departure races a new command; either serialized generation is valid.
 d=spawn(p,'echo LAST; cat >/dev/null');line(d)
 d.stdin.close();d.stdin=None
 f=spawn(p,'echo NEXT; cat >/dev/null');line(f);done(d);assert len(listed(p))==1
 f.stdin.close();f.stdin=None;done(f);assert not listed(p)
 # Escaped child survives session completion, and stop removes its box.
 pf(p,'start');ex(p,r'setsid sh -c "echo \$\$ > escaped; exec tail -f /dev/null" </dev/null >/dev/null 2>&1 &')
 wait_file(p/'escaped');ex(p,'kill -0 "$(cat escaped)"; echo ESCAPED-ALIVE')
 name=listed(p)[0]['name'];pf(p,'stop');pf(p,'box','stat',name,code=3);say('escaped descendant ended with checked box removal=True')
 # Owner death after ready: replacement follows checked orphan removal.
 pf(p,'start');old=listed(p)[0];os.kill(old['owner'],signal.SIGKILL)
 ex(p,'echo RECOVERED');assert not listed(p);say('owner-death orphan checked recovery=True old-generation='+old['labels']['dev.pinfold.generation'])
 # Host edits/commit do not change snapshot. Git eventually sees committed host state.
 pf(p,'start');before=snap(p);ex(p,'printf guest >> file; git log -1 --format=%H');assert snap(p)==before
 (p/'host-only').write_text('host');git(p,'add','file','host-only');git(p,'commit','-qm','host-between-joins');head=git(p,'rev-parse','HEAD');before=snap(p)
 ex(p,'echo JOIN-AFTER-HOST-COMMIT; git status --porcelain')
 r=ex(p,'git log -1 --format=%H');assert head in r.stdout.decode();assert snap(p)==before;say('host-commit join=True box-log-matches=True metadata-unchanged=True')
 # Nested linked worktree uses main configuration, owner and invoking cwd.
 wt=p/'linked';git(p,'worktree','add','-qb','linked',wt)
 ex(wt,'pwd; printf nested > nested-edit');assert (wt/'nested-edit').read_text()=='nested';assert len(listed(p))==1
 ex(wt,'git status --porcelain')
 outside=W/'outside';git(p,'worktree','add','-qb','external',outside)
 pf(outside,'--','true',code=125);assert not config(outside).exists()
 sep=W/'separate';sep.mkdir();raw(['git','init','-q','--separate-git-dir',W/'separate-meta',sep])
 pf(sep,'--','true',code=125);assert not config(sep).exists()
 # A writable worktree pointer must not run host fsmonitor outside the project.
 trap=W/'trap';trap.mkdir();git(trap,'init','-q');hook=trap/'monitor';hook.write_text('#!/bin/sh\necho invoked > '+shlex.quote(str(W/'HOST-EXECUTED'))+'\n');hook.chmod(0o755)
 git(trap,'config','core.fsmonitor',str(hook));ex(p,'printf "gitdir: '+str(trap/'.git')+'\\n" > linked/.git')
 pf(wt,'--','true',code=125);assert not (W/'HOST-EXECUTED').exists();say('hostile-pointer no-outside-Git-execution=True (outside-read confinement inspected)')
 # Changed mount and roots refuse; restore each and join same owner.
 saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"disk"'))
 ex(p,'echo MUST-NOT-RUN',code=125)
 pf(p,'push',code=1);pf(p,'fetch',code=1)
 config(p).write_text(saved);ex(p,'echo MOUNT-RESTORED')
 changed=dict(E,XDG_STATE_HOME=str(W/'different-state'));pf(p,'--','true',code=125,e=changed);ex(p,'echo ROOTS-RESTORED')
 for verb in ('push','fetch'):pf(p,verb,code=1)
 report=json.loads(pf(p,'config').stdout);assert report['project']['disks']['project'] is None
 names=[x['name'] for x in map(json.loads,pf(p,'disk','list','--json').stdout.splitlines())];assert not any(n.endswith('-project') for n in names)
 say('share report='+json.dumps(report['project'])+' no-project-disk=True no-host-remote='+str(not git(p,'for-each-ref','refs/remotes/host/')))
 # Fresh native seeds, all shared guidance. Existing user seed survives share->disk.
 ex(p,'for f in .pi/agent/APPEND_SYSTEM.md .claude/CLAUDE.md .codex/AGENTS.md; do grep -q "For a share project" "$HOME/$f" && grep -q "For a disk project" "$HOME/$f" || exit; done; echo THREE-GUIDANCE-SEEDS; printf USER-INSTRUCTION > "$HOME/.codex/AGENTS.md"')
 pf(p,'stop')
 for verb in ('push','fetch'):pf(p,verb,code=1)
 # Non-Git lifetime, gained Git topology, stop with missing config and hostile marker.
 n=setup('nongit',False);pf(n,'start');ex(n,'cat file; printf HOME-KEPT > "$HOME/kept"; printf host-edit > file')
 ex(n,'mkdir .git');ex(n,'true',code=125)
 ex_code=pf(n,'box','list','--label','dev.pinfold.project='+json.loads(pf(n,'config').stdout)['project']['id'])
 config(n).unlink();shutil.rmtree(n/'.git');(n/'.git').write_text('gitdir: /outside-hostile\n')
 pf(n,'stop');pf(n,'stop');assert (n/'file').read_text()=='host-edit';(n/'.git').unlink()
 pf(n,'config','edit');config(n).write_text('mounts = [{ host = ".", kind = "share" }]\nimage="localhost/pinfold/profile-default"\nallow=[]\n')
 ex(n,'cat "$HOME/kept"');say('nonGit home-persistence=True stop-hostile-marker-without-config=True')
 # Explicit Mac disk retains branch synchronization and private working files.
 disk=setup('disk',True,'disk');(disk/'untracked').write_text('excluded')
 report=json.loads(pf(disk,'config').stdout);assert report['project']['disks']['project']
 ex(disk,'test ! -e untracked; test "$(cat file)" = initial; printf disk-edit > file; git add file; git commit -qm guest; echo DISK-ISOLATED')
 assert (disk/'file').read_text()=='initial\n';pf(disk,'push');pf(disk,'fetch');assert git(disk,'log','-1','--format=%s','refs/remotes/pinfold/main')=='guest'
 say('disk report='+json.dumps(report['project'])+' private-copy-and-crossing=True')
 # Old project disk is retained across explicit share launch of the same project.
 saved=config(disk).read_text();config(disk).write_text(saved.replace('"disk"','"share"'));pf(disk,'start');pf(disk,'stop')
 names=[x['name'] for x in map(json.loads,pf(disk,'disk','list','--json').stdout.splitlines())];assert report['project']['disks']['project'] in names;say('share launch preserves existing old project volume=True')
 # Completed statuses retain precedence across final departure.
 for code in (0,23,125,126,127):ex(p,'exit '+str(code),code)
 say('PASS selected Mac project-share acceptance; no image build')
finally:
 for c in clients:
  if c.poll() is None:c.terminate();c.communicate(timeout=180)
 for p in projects:
  try:pf(p,'stop')
  except Exception as error:say('cleanup stop error='+str(error))
 for row in pf(W,'disk','list','--json').stdout.splitlines():
  pf(W,'disk','rm',json.loads(row)['name'],'--json')
 assert not pf(W,'disk','list','--json').stdout.strip()
 shutil.rmtree(W);say('fixture-removed='+str(not W.exists()))
