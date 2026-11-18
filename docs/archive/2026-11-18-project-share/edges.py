"""Additional topology/crossing/path refusals with corrected controls."""
import pathlib,re,ctypes,sys
exec(compile(pathlib.Path(__file__).with_name('apple.py').read_text().split('try:\n p=setup')[0],__file__,'exec'))
lib=ctypes.CDLL('/usr/lib/libproc.dylib');paused=set()
barriers=pathlib.Path(__file__).with_name('lifecycle.py').read_text()
exec(barriers[barriers.index('def note('):barriers.index('def locator(')],globals())
def token(r):return re.search(rb'^pinfold (?:--|start|push|fetch): ([^:]+):',r.stderr).group(1).decode()
def make(p,kind='disk',repository=True):
 p.parent.mkdir(parents=True,exist_ok=True);p.mkdir();projects.append(p);(p/'file').write_text('fixture\n')
 if repository:git(p,'init','-q','-b','main');git(p,'add','file');git(p,'commit','-qm','fixture')
 pf(p,'config','edit');config(p).write_text(f'mounts=[{{host=".",kind="{kind}"}}]\nimage="localhost/pinfold/profile-default"\nallow=[]\n');return p
def cross(p,verb,code=0,reason=None):
 r=pf(p,verb,code=code)
 if reason:assert re.search(rb'^pinfold (?:push|fetch): ([^:]+):',r.stderr).group(1).decode()==reason,r.stderr
 return json.loads(r.stdout) if r.stdout.strip() else None
def crossing_cases(mode):
 say('ROUND2 mode='+mode)
 parent=make(W/'parent');child=make(parent/'child');git(parent,'branch','parent-only');git(child,'branch','child-only')
 pf(child,'--','git','branch','child-only');pf(parent,'start');pf(parent,'--','git','branch','parent-only');parent_box=listed(parent)[0]['name']
 for verb in ('push','fetch'):
  result=cross(child,verb);keys=result['refs'];parent_selected=result['box']==parent_box
  if mode=='before':assert parent_selected and any(k.endswith('/parent-only') for k in keys) and not any(k.endswith('/child-only') for k in keys)
  else:assert not parent_selected and any(k.endswith('/child-only') for k in keys) and not any(k.endswith('/parent-only') for k in keys)
  say(f'root-selection {verb}: mode={mode} parent-selected={parent_selected} refs='+','.join(keys))
 child_refs=git(child,'for-each-ref','--format=%(refname)','refs/remotes/pinfold');assert ('child-only' in child_refs)==(mode!='before')
 say('child host receiving refs='+repr(child_refs));pf(parent,'stop')
 linked=make(W/'linked-disk');wt=linked/'worktree';git(linked,'worktree','add','-qb','linked',wt);pf(linked,'--','true')
 for kept in (False,True):
  if kept:pf(linked,'start')
  for verb in ('push','fetch'):
   cross(wt,verb,0 if mode=='before' else 1,None if mode=='before' else 'worktree')
  say(f'linked disk owner={kept}: '+('WRONG accepted' if mode=='before' else 'worktree refusal'))
 cross(linked,'push');cross(linked,'fetch');say('linked case direct-root positive control=True');pf(linked,'stop')
 scoped=make(W/'s/pinfold/work/state-descendant');pf(scoped,'start');cross(scoped,'push');cross(scoped,'fetch')
 saved=config(scoped).read_text()
 if mode!='main':config(scoped).write_text(saved.replace('"disk"','"share"'))
 for verb in ('push','fetch'):
  cross(scoped,verb,1 if mode=='before' else 0,'host-path-in-project' if mode=='before' else None)
 say('owner path rules: '+('WRONG share restrictions refused disk owner' if mode=='before' else 'disk rules permit descendant path')+(' (main control keeps supported disk configuration)' if mode=='main' else ' with file now share'))
 config(scoped).write_text(saved);pf(scoped,'stop')
 if mode!='main':
  shared=make(W/'shared','share',False);pf(shared,'start')
  for variant in ('ordinary','hostile-marker','config-disk'):
   if variant=='hostile-marker':ex(shared,'printf "gitdir: /outside-hostile\\n" > .git')
   if variant=='config-disk':config(shared).write_text(config(shared).read_text().replace('"share"','"disk"'))
   for verb in ('push','fetch'):cross(shared,verb,1,'share-no-crossing')
   say('share control '+variant+': push/fetch=share-no-crossing')
  pf(shared,'stop')
 say('PASS round2 '+mode+' crossing observations; no image built')

try:
 if len(sys.argv)==3:
  B=pathlib.Path(sys.argv[1]).resolve();mode=sys.argv[2];assert mode in ('before','main','after','absent')
  if mode=='absent':
   p=make(W/'s/pinfold/work/absent','share',False)
   for verb in ('push','fetch'):cross(p,verb,1,'share-no-crossing')
   say('absent owner: current share file decides; push/fetch=share-no-crossing')
  else:crossing_cases(mode)
 else:
  p=setup('topology',False);pf(p,'start');saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"disk"'))
  for v in ('push','fetch'):
   r=pf(p,v,code=1);assert b'share-no-crossing' in r.stderr
  config(p).write_text(saved);(p/'.git').write_text('gitdir: /outside-hostile\n')
  r=ex(p,'echo MUST-NOT-RUN',125);assert b'config-changed' in r.stderr
  (p/'.git').unlink();ex(p,'echo TOPOLOGY-RESTORED')
  pf(p,'stop');pf(p,'start');pf(p,'start');pf(p,'stop');pf(p,'stop')
  p=setup('path');before=snap(p);ex(p,'if printf refused >> .git/config; then exit 1; fi; printf allowed > ordinary; git log -1 --format=%s; echo GIT-READONLY-WORKFILE-WRITABLE');assert snap(p)==before;say('protected metadata unchanged after refused write=True');(p/'new-untracked-dir').mkdir();ex(p/'new-untracked-dir','pwd; echo CWD-AVAILABLE');saved=config(p).read_text();f=p/'config-source';f.write_text(saved);config(p).unlink();config(p).symlink_to(f)
  r=ex(p,'true',125);assert re.search(rb'^pinfold --: ([^:]+):',r.stderr).group(1)==b'host-path-in-project'
  config(p).unlink();config(p).write_text(saved);ex(p,'echo CONFIG-PATH-RESTORED')
  # Base protected roots are checked by the core, not the project-only extension.
  for root in ('state','cache'):
   e=dict(E,**{('XDG_STATE_HOME' if root=='state' else 'XDG_CACHE_HOME'):str(p/'protected')})
   r=pf(p,'--','true',code=125,e=e);assert re.search(rb'^pinfold --: ([^:]+):',r.stderr).group(1)==b'host-path-in-project'
  ex(p,'echo CORE-PATH-RESTORED')
  p=setup('no-config')
  outside=W/'external-worktree';git(p,'worktree','add','-qb','external',outside)
  sep=W/'separate-git';sep.mkdir();r=raw(['git','init','-q','--separate-git-dir',W/'git-metadata',sep]);assert r.returncode==0
  for folder in (outside,sep):
   r=pf(folder,'--','true',code=125);assert b'worktree' in r.stderr
   assert not config(folder).parent.exists() and not (W/'s/pinfold/projects'/config(folder).parent.name).exists();say('unsupported Git layout config-and-project-state-absent=True folder='+str(folder))
  config(p).unlink()
  for v in ('push','fetch'):
   r=pf(p,v,code=1);assert b'no-config' in r.stderr
  pf(p,'config','edit');config(p).write_text('mounts=[{host=".",kind="share"}]\nimage="localhost/pinfold/profile-default"\nallow=[]\n');ex(p,'echo CONFIG-RESTORED')
  # A real host-Git observer, with the exact old verification argv as control.
  p=setup('no-host-git',False);outside=W/'observer-repo';outside.mkdir();git(outside,'init','-q');git(outside,'commit','--allow-empty','-qm','fixture')
  bindir=W/'bin';bindir.mkdir();witness=W/'host-git-calls';wrapper=bindir/'git'
  wrapper.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> '+shlex.quote(str(witness))+'\nexec /usr/bin/git "$@"\n');wrapper.chmod(0o755)
  original_path=E['PATH'];E['PATH']=str(bindir)+':'+original_path
  r=raw(['git','-C',outside,'rev-parse','--show-toplevel']);assert r.returncode==0 and 'rev-parse --show-toplevel' in witness.read_text();say('host-Git observer positive-control fired=True argv=rev-parse --show-toplevel');witness.unlink()
  pf(p,'start');ex(p,'git init -q; printf %s '+shlex.quote(str(outside/'.git'))+' > .git/commondir')
  r=ex(p,'true',125);assert token(r)=='config-changed';pf(p,'stop')
  r=ex(p,'true',125);assert token(r)=='worktree';assert not witness.exists()
  (p/'.git/commondir').unlink();pf(p,'start');ex(p,'if printf x >> .git/config; then exit 1; fi; echo ADOPTED-GIT-PROTECTED');pf(p,'config')
  saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"disk"'));r=ex(p,'true',125);assert token(r)=='config-changed';config(p).write_text(saved);pf(p,'stop');assert not witness.exists();say('share startup/join/report/stop and post-stop adoption: host-Git observer calls=0')
  p=setup('hostile-crossing',False);pf(p,'start');ex(p,'printf "gitdir: /outside-hostile\\n" > .git');saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"disk"'))
  for v in ('push','fetch'):
   r=pf(p,v,code=1);assert token(r)=='share-no-crossing'
  config(p).write_text(saved);pf(p,'stop');assert not witness.exists();E['PATH']=original_path;say('hostile-marker crossing consults owner before discovery=True')
  p=setup('linked-roots');wt=p/'linked';git(p,'worktree','add','-qb','linked',wt);marker=(wt/'.git').read_text();pf(p,'start');ex(p,'printf "gitdir: /outside-hostile\\n" > linked/.git')
  bad=W/'bad-config';dest=bad/'pinfold/projects'/config(p).parent.name;dest.mkdir(parents=True);(dest/'pinfold.toml').write_text('invalid = [')
  r=pf(wt,'--','true',code=125,e=dict(E,XDG_CONFIG_HOME=str(bad)));assert token(r)=='config-changed'
  r=pf(wt,'--','true',code=125);assert token(r)=='worktree';(wt/'.git').write_text(marker);ex(wt,'pwd');pf(p,'stop');say('changed roots precede malformed config and worktree pointer=True')
  # Freeze the real image-inspection helper after a candidate's metadata read.
  # A previously admitted writer changes topology before Join/Keep can grant.
  for intent in ('join','keep'):
   p=setup('grant-'+intent,False);writer=spawn(p,'echo WRITER; while read cmd; do eval "$cmd"; echo MADE; done');line(writer)
   if intent=='join':c=spawn(p,'echo MUST-NOT-RUN')
   else:c=sp.Popen([str(B),'start'],cwd=p,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE);clients.append(c)
   q=select.kqueue();note(q,c.pid)
   native=barrier(q,c.pid,lambda a:'container image inspect ' in a,hold_parent=True)
   writer.stdin.write(b'mkdir .git\n');writer.stdin.flush();line(writer);resume(native);resume(c.pid)
   out,err=c.communicate(timeout=180);assert c.returncode==125 and not out and token(type('R',(),{'stderr':err})())=='config-changed';q.close()
   say('grant '+intent+': metadata captured without Git; guest gained .git before grant; single allowed outcome=125/config-changed; command absent=True')
   pf(p,'stop');writer.stdin.close();writer.stdin=None;done(writer,125)
  say('PASS nonGit running-owner crossing, hostile layout refusal, cwd, linked config, core roots, no-config controls')
finally:
 for pid in list(paused):resume(pid)
 for c in clients:
  if c.poll() is None:c.terminate();c.communicate(timeout=180)
 for p in projects:pf(p,'stop')
 for row in pf(W,'disk','list','--json').stdout.splitlines():pf(W,'disk','rm',json.loads(row)['name'],'--json')
 assert not pf(W,'disk','list','--json').stdout.strip();shutil.rmtree(W);say('edges fixture-removed='+str(not W.exists()))
