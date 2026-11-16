#!/usr/bin/env python3
"""PATH stand-in holds helper pipes after leader exit: BIN IMAGE [all|drain|stop|signal]."""
import json,os,pathlib,select,shlex,shutil,signal,subprocess as sp,sys,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
R=pathlib.Path(__file__).resolve().parents[3];ONLY=sys.argv[3] if len(sys.argv)>3 else 'all'
real=shutil.which('git',path=E['PATH']);assert real
P.mkdir();git(P,'init','-q','-b','review');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture');run(['config','edit'])
c=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
bindir=W/'bin';bindir.mkdir();settings=W/'helper.json';wrapper=bindir/'git'
wrapper.write_text(f'''#!{sys.executable}
import fcntl,json,os,pathlib,subprocess,sys
os.environ.update(GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
data=json.loads(pathlib.Path({str(settings)!r}).read_text())
if data['mode']=='real' or sys.argv[1:]!=['-C',{str(P)!r},'symbolic-ref','-q','HEAD']:
 os.execv({real!r},[{real!r},*sys.argv[1:]])
branch=subprocess.check_output([{real!r},*sys.argv[1:]])
leader=os.getpid();owner=os.getppid();child=os.fork()
if child==0:
 report=dict(leader=leader,child=os.getpid(),group=os.getpgrp(),owner=owner)
 pathlib.Path(data['report']).write_text(json.dumps(report))
 with open(data['ready'],'w') as f:f.write(json.dumps(report)+'\\n');f.flush()
 with open(data['hold'],'rb',buffering=0) as f:
  if f.read(1)==b'B':
   flags=fcntl.fcntl(2,fcntl.F_GETFL);fcntl.fcntl(2,fcntl.F_SETFL,flags|os.O_NONBLOCK);capacity=0
   try:
    while True:capacity+=os.write(2,b'F'*65536)
   except BlockingIOError:pass
   fcntl.fcntl(2,fcntl.F_SETFL,flags)
   with open(data['ready'],'w') as event:event.write(json.dumps(dict(event='filled',capacity=capacity))+'\\n');event.flush()
   payload=b'F'*(capacity+65537);written=0
   while payload:
    count=os.write(2,payload);written+=count;payload=payload[count:]
   with open(data['ready'],'w') as event:event.write(json.dumps(dict(event='drained',written=written))+'\\n');event.flush()
   f.read(1)
 os._exit(0)
with open(data['exit'],'rb') as f:assert f.read(1)==b'X'
os.write(1,branch);os._exit(0)
''');wrapper.chmod(0o755);E['PATH']=str(bindir)+':'+E['PATH'];settings.write_text('{"mode":"real"}')
class Observer:
 def __init__(self,pid):
  self.pid=pid;self.fd=os.pidfd_open(pid) if sys.platform=='linux' else None;self.q=None;self.dead=False
  if self.fd is None:
   self.q=select.kqueue();self.q.control([select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXIT)],0,0)
 def wait(self,timeout):
  self.dead=self.dead or (bool(select.select([self.fd],[],[],timeout)[0]) if self.fd is not None else bool(self.q.control([],1,timeout)))
  return self.dead
 def kill(self):
  if self.wait(0):return
  if self.fd is not None:signal.pidfd_send_signal(self.fd,signal.SIGKILL)
  else:os.kill(self.pid,signal.SIGKILL)
 def close(self):
  if self.fd is not None:os.close(self.fd)
  if self.q is not None:self.q.close()
def case(mode):
 paths={key:W/(mode+'.'+key) for key in ('ready','hold','exit','drain','go','report','status','out','err','gdb','debug')}
 for key in ('ready','hold','exit','drain','go'):os.mkfifo(paths[key])
 keep={key:os.open(paths[key],os.O_RDWR|os.O_NONBLOCK|os.O_CLOEXEC) for key in ('ready','hold','exit','drain','go')}
 settings.write_text(json.dumps(dict(mode='fork',**{key:str(paths[key]) for key in ('ready','hold','exit','report')})))
 process=stopper=None;observed={};handles=[];owner_paused=False;owner=None
 def read(key):
  assert select.select([keep[key]],[],[],90)[0],'required '+key+' fixture marker absent'
  return json.loads(os.read(keep[key],65536))
 try:
  if mode=='drain' or sys.platform=='darwin':process=sp.Popen([B,'--','printf','DRAIN-CONTROL'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
  else:
   source=R/'crates/pinfold/src/core/process.rs';found=[i+1 for i,line in enumerate(source.read_text().splitlines()) if line.strip()=='input_failed |= !input.is_empty();'];assert len(found)==1
   paths['gdb'].write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
handle SIGPIPE nostop noprint pass
python
import gdb,json,os,pathlib,threading
starter=None
status_thread=None
def status():
 pid,value=os.waitpid(starter,0);pathlib.Path({str(paths['status'])!r}).write_text(str(os.waitstatus_to_exitcode(value)))
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  global status_thread
  if starter is not None and gdb.selected_inferior().pid!=starter:
   self.enabled=False;gdb.execute('set follow-fork-mode parent');status_thread=threading.Thread(target=status);status_thread.start()
  return False
class Draining(gdb.Breakpoint):
 def stop(self):
  if not pathlib.Path({str(paths['report'])!r}).exists():return False
  data=json.loads(pathlib.Path({str(paths['report'])!r}).read_text())
  if starter is None or gdb.selected_inferior().pid==starter:return False
  assert pathlib.Path('/proc/'+str(data['leader'])+'/stat').read_text().split(') ',1)[1].startswith('Z'),'leader was reaped before pipe drain'
  self.enabled=False
  data.update(owner=gdb.selected_inferior().pid,starter=starter)
  with open({str(paths['drain'])!r},'w') as f:f.write(json.dumps(data)+'\\n');f.flush()
  with open({str(paths['go'])!r},'rb') as f:assert f.read(1)==b'G'
  return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Draining({str(source)+':'+str(found[0])!r})
end
run -- printf MUST-NOT-RUN > {shlex.quote(str(paths['out']))} 2> {shlex.quote(str(paths['err']))}
python
if status_thread is not None:
 status_thread.join(90);assert not status_thread.is_alive(),'starter survived owner completion'
end
''')
   with paths['debug'].open('wb') as log:process=sp.Popen(['gdb','-nx','-q','-batch','-x',str(paths['gdb']),'--args',B],cwd=P,env=E,stdin=sp.DEVNULL,stdout=log,stderr=sp.STDOUT)
  observed=read('ready');assert observed['group']==observed['leader'] and os.getpgid(observed['child'])==observed['leader']
  leader,descendant=Observer(observed['leader']),Observer(observed['child']);handles=[leader,descendant]
  assert not descendant.wait(0);os.write(keep['exit'],b'X');assert leader.wait(30),'stand-in leader did not exit'
  began=time.monotonic()
  if mode=='drain':
   out,err=process.communicate(timeout=90);assert process.returncode==0 and out==b'DRAIN-CONTROL',(process.returncode,out,err)
  elif sys.platform=='linux':
   gate=read('drain');assert gate['leader']==observed['leader'] and gate['child']==observed['child'];assert not descendant.wait(0)
   owner=Observer(gate['owner']);handles.append(owner)
   if mode=='stop':stopper=sp.Popen([B,'stop'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
   else:os.kill(gate['starter'],signal.SIGTERM)
   os.write(keep['go'],b'G')
   if stopper is not None:
    out,err=stopper.communicate(timeout=90);assert stopper.returncode==0,(out,err)
   process.communicate(timeout=90);assert process.returncode==0,paths['debug'].read_bytes()[-1000:]
   expected=125 if mode=='stop' else 143
   assert paths['status'].read_text()==str(expected) and paths['out'].read_bytes()==b''
   assert owner.wait(10) and not locator(P).exists()
  else:
   owner=Observer(observed['owner']);handles.append(owner);os.kill(owner.pid,signal.SIGSTOP);owner_paused=True
   deadline=time.monotonic()+2
   while b'T' not in sp.run(['ps','-p',str(owner.pid),'-o','state='],capture_output=True,check=True).stdout:assert time.monotonic()<deadline,'owner did not stop'
   os.write(keep['hold'],b'B');filled=read('ready');assert filled['event']=='filled' and filled['capacity']>0
   os.kill(owner.pid,signal.SIGCONT);owner_paused=False;drained=read('ready')
   assert drained==dict(event='drained',written=filled['capacity']+65537)
   assert b'Z' in sp.run(['ps','-p',str(leader.pid),'-o','state='],capture_output=True,check=True).stdout,'leader was reaped during drain'
   assert not descendant.wait(0) and process.poll() is None,'post-exit drain window ended before cancellation'
   # A full pipe plus this completed write forces more than one 64KiB drain quantum.
   if mode=='stop':
    stopper=sp.Popen([B,'stop'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
    out,err=stopper.communicate(timeout=90);assert stopper.returncode==0,(out,err)
   else:process.send_signal(signal.SIGTERM)
   out,err=process.communicate(timeout=90);expected=125 if mode=='stop' else 143
   assert process.returncode==expected and out==b'',(process.returncode,out,err)
   assert owner.wait(10) and not locator(P).exists()
  assert descendant.wait(10),'pipe-holding descendant survived completed helper cleanup'
  client_status=process.returncode if mode=='drain' or sys.platform=='darwin' else int(paths['status'].read_text())
  proof='normal one-second drain expired' if mode=='drain' else ('GDB proved leader zombie after exit observation' if sys.platform=='linux' else f'private stderr capacity={filled["capacity"]}; write={drained["written"]} completed; leader still zombie')
  emit(f'helper-{mode}: stand-in leader exited0; child held stdout/stderr in leader group; '+proof+f'; descendant kernel exit before return check; seconds={time.monotonic()-began:.2f}; client={client_status}; CMD='+('DRAIN-CONTROL/0' if mode=='drain' else 'absent'))
 finally:
  if owner_paused and owner is not None:
   try:os.kill(owner.pid,signal.SIGCONT)
   except ProcessLookupError:pass
  os.write(keep['exit'],b'X');os.write(keep['go'],b'G')
  for handle in reversed(handles):handle.kill();handle.close()
  if process is not None and process.poll() is None:process.terminate();process.communicate(timeout=30)
  if stopper is not None and stopper.poll() is None:stopper.terminate();stopper.communicate(timeout=30)
  for fd in keep.values():os.close(fd)
  settings.write_text('{"mode":"real"}');run(['stop'])
 assert run(['--','printf','REAL-GIT-CONTROL']).stdout==b'REAL-GIT-CONTROL'
 emit(f'helper-{mode} control: real Git delegated; next start=0/REAL-GIT-CONTROL')
try:
 for mode in ('drain','stop','signal'):
  if ONLY not in ('all',mode):continue
  case(mode)
finally:
 settings.write_text('{"mode":"real"}');run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup exact own helper descendants/boxes/disks=True')
