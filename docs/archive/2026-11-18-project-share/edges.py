"""Additional topology/crossing/path refusals with corrected controls."""
import pathlib
exec(compile(pathlib.Path(__file__).with_name('apple.py').read_text().split('try:\n p=setup')[0],__file__,'exec'))
try:
 p=setup('topology',False);pf(p,'start');saved=config(p).read_text();config(p).write_text(saved.replace('"share"','"disk"'))
 for v in ('push','fetch'):
  r=pf(p,v,code=1);assert b'share-no-crossing' in r.stderr
 config(p).write_text(saved);(p/'.git').write_text('gitdir: /outside-hostile\n')
 r=ex(p,'echo MUST-NOT-RUN',125);assert b'config-changed' in r.stderr
 (p/'.git').unlink();ex(p,'echo TOPOLOGY-RESTORED')
 pf(p,'stop');pf(p,'start');pf(p,'start');pf(p,'stop');pf(p,'stop')
 p=setup('path');before=snap(p);ex(p,'if printf refused >> .git/config; then exit 1; fi; printf allowed > ordinary; git log -1 --format=%s; echo GIT-READONLY-WORKFILE-WRITABLE');assert snap(p)==before;say('protected metadata unchanged after refused write=True');(p/'new-untracked-dir').mkdir();ex(p/'new-untracked-dir','pwd; echo CWD-AVAILABLE');saved=config(p).read_text();f=p/'config-source';f.write_text(saved);config(p).unlink();config(p).symlink_to(f)
 r=ex(p,'true',125);assert b'host-path-in-project' in r.stderr
 config(p).unlink();config(p).write_text(saved);ex(p,'echo CONFIG-PATH-RESTORED')
 # Base protected roots are checked by the core, not the project-only extension.
 for root in ('state','cache'):
  e=dict(E,**{('XDG_STATE_HOME' if root=='state' else 'XDG_CACHE_HOME'):str(p/'protected')})
  r=pf(p,'--','true',code=125,e=e);assert b'host-path-in-project' in r.stderr
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
 say('PASS nonGit running-owner crossing, hostile layout refusal, cwd, linked config, core roots, no-config controls')
finally:
 for p in projects:pf(p,'stop')
 for row in pf(W,'disk','list','--json').stdout.splitlines():pf(W,'disk','rm',json.loads(row)['name'],'--json')
 assert not pf(W,'disk','list','--json').stdout.strip();shutil.rmtree(W);say('edges fixture-removed='+str(not W.exists()))
