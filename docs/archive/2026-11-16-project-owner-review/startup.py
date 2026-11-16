#!/usr/bin/env python3
"""Real held startup workers: startup.py BIN IMAGE; no runtime stubs."""
import json,os,pathlib,select,shlex,signal,subprocess as sp,sys,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner/crashes.py'
exec(compile(old.read_text().split('def crash(')[0],str(old),'exec'),globals())

def absent():
 assert not L.exists(),'startup left its locator'

def control():
 assert run(['--','sh','-c','printf STARTUP-CONTROL']).stdout==b'STARTUP-CONTROL';run(['stop']);absent()

def linux(mode):
 ready=W/(mode+'.ready');os.mkfifo(ready);pipe=os.open(ready,os.O_RDWR|os.O_NONBLOCK);script=W/(mode+'.gdb');out=W/(mode+'.out');err=W/(mode+'.err');cancelled=W/(mode+'.cancelled');killing=W/(mode+'.killing')
 vm=R/'crates/pinfold/src/core/runtime/firecracker/vm.rs';lines=vm.read_text().splitlines();cancel_line=next(i+1 for i,line in enumerate(lines) if line.strip()=='sent?;');kill_line=next(i+1 for i,line in enumerate(lines) if 'self.vmm.kill()?;' in line)
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,signal,hashlib
starter=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:
   self.enabled=False;gdb.execute('set follow-fork-mode parent')
  return False
class Gate(gdb.Breakpoint):
 def stop(self):
  owner=gdb.selected_inferior().pid
  if starter is None or owner==starter:return False
  self.enabled=False
  loc=json.loads(pathlib.Path({str(L)!r}).read_text());record=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');claim=json.loads(record.read_text());vmm=int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0])
  os.kill(vmm,signal.SIGSTOP if {mode!r}=='boot-stop' else signal.SIGKILL)
  with open({str(ready)!r},'w') as f:f.write(json.dumps({{'owner':owner,'vmm':vmm,'generation':claim['generation']}})+'\\n');f.flush()
  return False
class Cancelled(gdb.Breakpoint):
 def stop(self):
  pathlib.Path({str(cancelled)!r}).touch();return False
class Killing(gdb.Breakpoint):
 def stop(self):
  assert pathlib.Path({str(cancelled)!r}).exists(),'BOOT still blocked when down kills its VMM'
  pathlib.Path({str(killing)!r}).touch();return False
if {mode!r}=='boot-stop':
 Cancelled({str(vm)!r}+':'+str({cancel_line}))
 Killing({str(vm)!r}+':'+str({kill_line}))
Spawn('pinfold::pi::session::spawn_owner')
Main('pinfold::main')
Gate('pinfold::core::runtime::firecracker::vm::Boot::run')
end
run -- sh -c 'printf MUST-NOT-RUN' > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
''')
 debugger=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=sp.PIPE,stderr=sp.PIPE);held=None
 try:
  assert select.select([pipe],[],[],100)[0],'required BOOT worker barrier not hit';held=json.loads(os.read(pipe,4096));handle=os.pidfd_open(held['vmm']) if mode=='boot-stop' else None
  began=time.monotonic()
  if mode=='boot-stop':run(['stop'])
  output,errors=debugger.communicate(timeout=100);assert debugger.returncode==0,(output+errors)[-500:]
  assert out.read_bytes()==b'' and err.read_bytes(),'incomplete startup admitted CMD or lost its diagnostic'
  if handle is not None:assert select.select([handle],[],[],10)[0] and cancelled.exists() and killing.exists(),'VMM survived or BOOT cancellation ordering absent';os.close(handle)
  with open(LOCK,'rb') as lock:
   import fcntl
   fcntl.flock(lock,fcntl.LOCK_EX)
  absent();control();emit(f'{mode}: required BOOT barrier hit; real VMM '+('SIGSTOP then stop=0' if mode=='boot-stop' else 'SIGKILL')+f'; CMD=absent; startup diagnostics=present; BOOT joined; restart=STARTUP-CONTROL; seconds={time.monotonic()-began:.2f}')
 finally:
  os.close(pipe)
  if debugger.poll() is None:debugger.terminate();debugger.communicate(timeout=10)
  if held is not None:
   try:os.kill(held['vmm'],signal.SIGCONT)
   except ProcessLookupError:pass

def mac():
 profile=W/'c/pinfold/profiles/default';(profile/'home').mkdir(parents=True);(profile/'pinfold.toml').write_text('')
 # Missing seed copy is a real native exec with a large pipe transfer.
 with (profile/'home/probe-large-seed').open('wb') as f:f.write(b'S'*(64*1024*1024))
 starter=sp.Popen([B,'--','sh','-c','printf MUST-NOT-RUN'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE);native=None;queue=select.kqueue();began=time.monotonic()
 try:
  deadline=time.monotonic()+100
  while native is None:
   assert time.monotonic()<deadline,'seed-copy helper barrier not observed'
   if L.exists():
    rows=[tuple(map(int,line.split())) for line in sp.run(['ps','-axo','pid=,ppid='],capture_output=True,check=True).stdout.splitlines()]
    owners=[pid for pid,ppid in rows if ppid==starter.pid]
    for owner in owners:
     for pid,ppid in rows:
      if ppid!=owner:continue
      args=sp.run(['ps','-p',str(pid),'-o','command='],capture_output=True,check=False).stdout
      if b'init receive --missing' in args:
       event=select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXIT)
       queue.control([event],0,0);os.kill(pid,signal.SIGSTOP);state=sp.run(['ps','-p',str(pid),'-o','state='],capture_output=True,check=True).stdout;assert b'T' in state,'native helper did not stop';native=pid;break
   assert starter.poll() is None,'startup ended before helper barrier'
  run(['stop']);out,err=starter.communicate(timeout=100);assert starter.returncode==125 and not out and err,'stop failed to reject startup CMD'
  assert queue.control([],1,10),'held native helper was not reaped';absent();(profile/'home/probe-large-seed').unlink();control()
  emit(f'finish-start: seed-copy native helper SIGSTOP confirmed; stop=0 starter=125 CMD=absent; helper exited; restart=STARTUP-CONTROL; seconds={time.monotonic()-began:.2f}')
 finally:
  queue.close()
  if native is not None:
   try:os.kill(native,signal.SIGCONT)
   except ProcessLookupError:pass
  if starter.poll() is None:starter.terminate();starter.communicate(timeout=100)
try:
 if sys.platform=='linux':linux('boot-stop');linux('boot-vmm-exit')
 else:mac()
finally:
 try:run(['stop'])
 finally:run(['disk','rm','--all'])
 emit('fixture='+str(W))
