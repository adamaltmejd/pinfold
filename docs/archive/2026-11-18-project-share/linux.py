"""Run in leased Debian VM; public CLI, no real credentials."""
import hashlib,json,os,pathlib,shutil,subprocess as sp,tempfile,sys
B=pathlib.Path.home()/'pinfold/target/x86_64-unknown-linux-musl/debug/pinfold'
W=pathlib.Path(tempfile.mkdtemp(prefix='pf280.',dir='/tmp'));P=W/'project';P.mkdir();(P/'file').write_text('initial\n')
E={k:os.environ[k] for k in ('PATH','HOME')};E.update(PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),VISUAL='/usr/bin/true',EDITOR='/usr/bin/true',GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
def say(s):print(str(s).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user').replace(str(B),'$BIN'),flush=True)
def raw(a,e=E,**kw):return sp.run(list(map(str,a)),cwd=P,env=e,capture_output=True,timeout=900,**kw)
def pf(*a,e=E,code=0):
 r=raw([B,*a],e,input=b'');say(f'CLI {a!r} rc={r.returncode} out={r.stdout.decode(errors="replace")[-1200:]!r} err={r.stderr.decode(errors="replace")[-700:]!r}')
 assert r.returncode==code;return r
for args in [('init','-q','-b','main'),('add','file'),('-c','user.name=user','-c','user.email=user@fixture.invalid','commit','-qm','initial')]:
 r=raw(['/usr/bin/git',*args]);assert r.returncode==0,r.stderr
stamp=W/'s/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch()
ident='project-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12];C=W/'c/pinfold/projects'/ident/'pinfold.toml'
def inventory(paths):
 return [(str(p),[(str(x.relative_to(p)),hashlib.sha256(x.read_bytes()).hexdigest()) for x in sorted(p.rglob('*')) if x.is_file()]) if p.exists() else (str(p),None) for p in paths]
try:
 pf('config','edit');assert 'kind = "disk"' in C.read_text();say('Linux config edit default=disk')
 C.write_text('mounts=[{host=".",kind="share"}]\nimage="unavailable"\n')
 refusal=dict(E,XDG_STATE_HOME=str(W/'absent-state'),XDG_CACHE_HOME=str(W/'absent-cache'),PATH='/nonexistent')
 roots=[W/'c',W/'absent-state',W/'absent-cache',pathlib.Path('/tmp')/f'pinfold-{os.getuid()}'];before=inventory(roots)
 say('Linux refusal roots before='+repr([(str(p),p.exists()) for p in roots])+' maintenance-due=True runtime-PATH=/nonexistent')
 for args in [('--','true'),('start',),('push',),('fetch',)]:
  r=pf(*args,e=refusal,code=125 if args[0] in ('--','start') else 1);assert b'spec' in r.stderr and b'share mounts are unsupported on Linux' in r.stderr
 assert before==inventory(roots);say('Linux share all-entrypoints spec refusal: config/state/cache/ownership inventories unchanged=True')
 if '--refusal-only' in sys.argv:
  C.write_text('mounts=[{host=".",kind="disk"}]\nimage="fixture-unavailable"\nallow=[]\n')
  report=json.loads(pf('config').stdout);assert report['project']['mount']['kind']=='disk'
  r=pf('--','true',code=125);assert b'image-missing' in r.stderr
  say('PASS fix-round Linux refusal/default-disk/config control; running owner and startup fast-forward UNESTABLISHED (no existing image; no image built)')
  sys.exit(0)
 context=W/'context';context.mkdir();recipe=W/'Containerfile';recipe.write_text('FROM debian:trixie-slim\nRUN apt-get update -qq && apt-get install -y -qq --no-install-recommends git && rm -rf /var/lib/apt/lists/*\n')
 ref=os.environ.get('PF280_LINUX_IMAGE')
 if not ref:
  image=pf('image','build','probe-project-share280-linux','--containerfile',str(recipe),'--context',str(context));ref=json.loads(image.stdout.splitlines()[-1])['ref']
 else:say('Reused fixture image='+ref)
 C.write_text('mounts=[{host=".",kind="disk"}]\nimage='+json.dumps(ref)+'\nallow=[]\n')
 report=json.loads(pf('config').stdout);assert report['project']['mount']['kind']=='disk' and report['project']['disks']['project'];say('Linux disk config report='+json.dumps(report['project']))
 pf('--','sh','-c','test "$(cat file)" = initial; printf guest > file; git -c user.name=user -c user.email=user@fixture.invalid add file; git -c user.name=user -c user.email=user@fixture.invalid commit -qm guest; printf home > "$HOME/kept"; echo DISK-FIRST')
 assert (P/'file').read_text()=='initial\n'
 pf('--','sh','-c','test "$(cat file)" = guest; test "$(cat "$HOME/kept")" = home; echo DISK-REUSED')
 pf('push');pf('fetch');r=raw(['/usr/bin/git','log','-1','--format=%s','refs/remotes/pinfold/main']);assert r.stdout.strip()==b'guest';say('Linux existing disk isolation/reuse/sync/push/fetch=True')
 say('PASS Linux project-share refusal and existing disk behavior')
finally:
 pf('stop')
 for row in pf('disk','list','--json').stdout.splitlines():pf('disk','rm',json.loads(row)['name'],'--json')
 assert not pf('disk','list','--json').stdout.strip()
 assert not pf('box','list','--label','dev.pinfold.project='+ident).stdout.strip()
 shutil.rmtree(W);say('Linux fixture-removed='+str(not W.exists()))
