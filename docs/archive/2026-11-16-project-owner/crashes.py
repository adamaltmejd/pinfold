#!/usr/bin/env python3
"""Linux GDB production barriers: crashes.py BIN IMAGE [case|all]."""
import fcntl,hashlib,json,os,pathlib,select,shlex,signal,subprocess as sp,sys,tempfile
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];ONLY=sys.argv[3] if len(sys.argv)>3 else 'all'
R=pathlib.Path(__file__).resolve().parents[3];W=pathlib.Path(tempfile.mkdtemp(prefix='pfoc.',dir='/tmp')).resolve()
P=W/'p';P.mkdir();H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
def emit(s):print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(str(R),'$REPO').replace(str(pathlib.Path.home()),'/home/user'),flush=True)
def run(a,code=0):
 r=sp.run([B]+a,cwd=P,env=E,input=b'',capture_output=True,timeout=180)
 emit(f'{a!r} exit={r.returncode} out={r.stdout[:100]!r} err={r.stderr[:250]!r}')
 assert r.returncode==code,'CLI status mismatch';return r
sp.run(['git','init','-q',str(P)],check=True)
sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
C=W/'c/pinfold/projects'/ID/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
KEY=hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest();L=H/(KEY+'.project')
LOCK=H/(hashlib.sha256(('project-launch:'+KEY).encode()).hexdigest()+'.lock')
SESSION=R/'crates/pinfold/src/pi/session.rs';OWNER=R/'crates/pinfold/src/core/ownership.rs'
JAIL=R/'crates/pinfold/src/core/runtime/firecracker/jail.rs';FC=R/'crates/pinfold/src/core/runtime/firecracker/mod.rs'
def anchor(path,text,owner=False,nth=None):
 lines=path.read_text().splitlines();start=next(i for i,l in enumerate(lines) if 'pub(crate) fn owner(' in l) if owner else 0
 found=[i+1 for i,l in enumerate(lines) if i>=start and text in l]
 assert found and (nth is not None or len(found)==1),'source anchor absent/ambiguous: '+text
 return f'{path}:{found[nth or 0]}'
def crash(label,location,starter=False):
 observation=W/(label+'.json');commands=W/(label+'.gdb');out=W/(label+'.out');err=W/(label+'.err')
 script=f'''set pagination off
set confirm off
set print thread-events off
set breakpoint pending on
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,signal,select,fcntl,subprocess,time
starter=None
contender=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid
  self.enabled=False
  gdb.execute('set follow-fork-mode child')
  return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:
   gdb.execute('set follow-fork-mode parent')
   self.enabled=False
  return False
class Crash(gdb.Breakpoint):
 def stop(self):
  global contender
  pid=gdb.selected_inferior().pid
  if starter is None or pid==starter:return False
  locator=pathlib.Path({str(L)!r})
  loc=json.loads(locator.read_text()) if locator.exists() else None
  record=locator.parent/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner') if loc else None
  r=json.loads(record.read_text()) if record and record.exists() else None
  if {label!r} in ('identity-before-write','identity-before-sync','identity-synced','gate-released') and not r:return False
  self.enabled=False
  generation=r['generation'] if r else None
  state=pathlib.Path(r['state_dir']) if r else None
  terminal=pathlib.Path({str(W/'s/pinfold/runs')!r})/generation/'terminal.json' if generation else None
  result={{'case':{label!r},'owner':pid,'starter':starter,'locator':bool(loc),'claim':bool(r),'generation':generation,'retiring':r['retiring'] if r else None,'identity_exists':bool(state and (state/'vmm').exists()),'terminal':terminal.read_text() if terminal and terminal.exists() else None,'children':[]}}
  handles=[]
  for task in pathlib.Path('/proc/'+str(pid)+'/task').iterdir():
   for child in task.joinpath('children').read_text().split():
    child=int(child)
    try:handles.append(os.pidfd_open(child))
    except ProcessLookupError:continue
    result['children'].append(child)
  if {starter!r}:
   handle=os.pidfd_open(starter);os.kill(starter,signal.SIGKILL)
   assert select.select([handle],[],[],30)[0],'starter survived SIGKILL';os.close(handle)
   with open({str(LOCK)!r},'rb') as lock:
    try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    except BlockingIOError:result['inherited_lock_held']=True
    else:raise AssertionError('starter death released owner launch lock')
   contender=subprocess.Popen([{B!r},'--','sh','-c','printf INHERITED-RECOVERY'],cwd={str(P)!r},stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
   assert contender.poll() is None,'contender exited at owner barrier'
  else:
   os.kill(pid,signal.SIGKILL)
   for handle in handles:assert select.select([handle],[],[],30)[0],'child survived owner death'
   result['children_ended']=True
  for handle in handles:os.close(handle)
  pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
  return not {starter!r}
import hashlib
Spawn('pinfold::pi::session::spawn_owner')
Main('pinfold::main')
Crash({location!r})
end
run -- sh -c 'printf ADMITTED' > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
{'' if starter else 'continue'}
python
if contender is not None:
 out,err=contender.communicate(timeout=180)
 assert contender.returncode==0 and out==b'INHERITED-RECOVERY','contender did not recover after cancelled bootstrap'
 r=json.loads(pathlib.Path({str(observation)!r}).read_text());r['contender_recovered']=True
 pathlib.Path({str(observation)!r}).write_text(json.dumps(r))
end
'''
 commands.write_text(script)
 result=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',B],cwd=P,env=E,capture_output=True,timeout=240)
 if not observation.exists():
  emit((result.stdout+result.stderr).decode(errors='replace')[-900:]);raise AssertionError('required breakpoint not hit: '+label)
 observed=json.loads(observation.read_text());owner=observed['owner']
 try:
  handle=os.pidfd_open(owner);assert select.select([handle],[],[],30)[0],'owner did not end';os.close(handle)
 except ProcessLookupError:pass
 if starter:assert observed.get('inherited_lock_held') and observed.get('contender_recovered'),'inherited-lock case incomplete'
 else:assert observed.get('children_ended'),'crash did not verify child death'
 generation=observed['generation']
 if generation:
  terminal=W/'s/pinfold/runs'/generation/'terminal.json'
  assert (terminal.read_text() if terminal.exists() else None)==observed['terminal'],'crash fabricated/changed terminal record'
 if label=='before-locator':assert not observed['locator'] and not observed['claim']
 if label=='after-locator':assert observed['locator'] and not observed['claim']
 if label=='after-claim':assert observed['claim']
 if label=='after-retirement':assert observed['retiring']
 if label=='after-identity-delete':assert observed['retiring'] and not observed['identity_exists']
 assert run(['--','sh','-c','printf RECOVERED']).stdout==b'RECOVERED'
 run(['stop']);assert not L.exists(),'locator survived completed recovery'
 if generation:
  assert (terminal.read_text() if terminal.exists() else None)==observed['terminal'],'recovery inferred a terminal record'
 emit(f'{label}: barrier hit; claim={observed["claim"]} retiring={observed["retiring"]} children={len(observed["children"])} terminal-preserved=True checked-restart=True'+(' inherited-lock=True contender=True' if starter else ''))
cases=[
 ('before-locator',SESSION,'locator.publish()?;',True,0),
 ('after-locator',SESSION,'state::record_run(&root)?;',True,None),
 ('after-claim',OWNER,'self.sockets = Some(Sockets::create(self.generation())?);',False,None),
 ('identity-before-write',JAIL,'super::record_vmm(state_dir, pid)?;',False,None),
 ('identity-before-sync',FC,'file.sync_all()?;',False,None),
 ('identity-synced',JAIL,'match nix::unistd::write(&alive_write, &[1])',False,None),
 ('gate-released',JAIL,'let mut filled = 0;',False,None),
 ('before-retirement',OWNER,'mark_retiring(&mut current)?;',False,None),
 ('after-retirement',OWNER,'held.retiring = true;',False,0),
 ('after-identity-delete',OWNER,'sockets::remove_socket(&socket(name)?)?;',False,None),
]
try:
 selected=[c for c in cases if ONLY=='all' or c[0]==ONLY]
 assert selected or ONLY=='starter-death','unknown case'
 for label,file,text,own,nth in selected:crash(label,anchor(file,text,own,nth))
 if ONLY in ('all','starter-death'):crash('starter-death',anchor(OWNER,'self.sockets = Some(Sockets::create(self.generation())?);'),True)
 emit('PASS selected real GDB barriers; power-loss and unlisted instruction windows not exercised')
finally:
 try:run(['stop'])
 except Exception as error:emit('cleanup-stop-failed='+str(error))
 try:run(['disk','rm','--all'])
 except Exception as error:emit('cleanup-disk-failed='+str(error))
 emit('fixture='+str(W))
