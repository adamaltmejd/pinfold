#!/usr/bin/env python3
"""Real signal during leave: BIN IMAGE; setup Err and observed 0/23 controls."""
import ctypes,fcntl,hashlib,json,os,pathlib,select,shlex,signal,subprocess as sp,sys,termios,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
R=pathlib.Path(__file__).resolve().parents[3];SESSION=R/'crates/pinfold/src/pi/session.rs'
P.mkdir();git(P,'init','-q','-b','rereview');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture')
run(['config','edit']);C=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
def control():
 assert run(['--','sh','-c','printf LEAVE-CONTROL; sync']).stdout==b'LEAVE-CONTROL';run(['stop']);assert not locator(P).exists()
def anchor(text):
 found=[i+1 for i,line in enumerate(SESSION.read_text().splitlines()) if text in line];assert len(found)==1,(text,found);return str(SESSION)+':'+str(found[0])
def linux(code):
 label='setup' if code is None else str(code);ready,go,ack=[W/(label+x) for x in ('.ready','.go','.ack')]
 for p in (ready,go,ack):os.mkfifo(p)
 pipes=[os.open(p,os.O_RDWR|os.O_NONBLOCK) for p in (ready,ack)];script=W/(label+'.gdb');out=W/(label+'.out');err=W/(label+'.err');exitfile=W/(label+'.exit')
 command='printf MUST-NOT-RUN' if code is None else f'printf COMMAND-COMPLETE; sync; exit {code}'
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
handle SIGTERM nostop noprint pass
python
import gdb,hashlib,json,os,pathlib,signal,time
transport_held=False
transport_started=None
class BeforeExec(gdb.Breakpoint):
 def stop(self):
  global transport_held,transport_started
  self.enabled=False
  loc=json.loads(pathlib.Path({str(locator(P))!r}).read_text());record=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');claim=json.loads(record.read_text());vmm=int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]);os.kill(vmm,signal.SIGSTOP)
  deadline=time.monotonic()+2
  while not pathlib.Path('/proc/'+str(vmm)+'/status').read_text().split('State:',1)[1].lstrip().startswith('T'):assert time.monotonic()<deadline,'VMM did not stop before exec transport'
  transport_held=True;transport_started=time.monotonic()
  return False
if {code is None!r}:BeforeExec('pinfold::core::runtime::firecracker::vsock::exec')
class Leave(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  assert not {code is None!r} or transport_held,'before-exec transport barrier absent'
  loc=json.loads(pathlib.Path({str(locator(P))!r}).read_text());record=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');claim=json.loads(record.read_text());vmm=int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]);os.kill(vmm,signal.SIGSTOP)
  with open({str(ready)!r},'w') as f:f.write(json.dumps(dict(client=gdb.selected_inferior().pid,vmm=vmm,owner=int(claim['described']['labels']['dev.pinfold.owner']),generation=loc['generation'],transport_wait=None if transport_started is None else time.monotonic()-transport_started))+'\\n');f.flush()
  with open({str(go)!r},'rb') as f:assert f.read(1)==b'G'
  return False
class Ack(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  with open({str(ack)!r},'w') as f:f.write('D');f.flush()
  return False
Leave({anchor('let leave = lease.leave(signals).await;')!r});Ack({anchor('acknowledged = true;')!r})
def exited(event):pathlib.Path({str(exitfile)!r}).write_text(str(event.exit_code))
gdb.events.exited.connect(exited)
end
run -- sh -c {shlex.quote(command)} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
''')
 debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=sp.PIPE,stderr=sp.PIPE);observed=None;owner_fd=None
 try:
  assert select.select([pipes[0]],[],[],90)[0],'before-leave gate absent';observed=json.loads(os.read(pipes[0],4096));owner_fd=os.pidfd_open(observed['owner'])
  with go.open('wb',buffering=0) as f:f.write(b'G')
  assert select.select([pipes[1]],[],[],10)[0] and os.read(pipes[1],1)==b'D','Draining acknowledgement absent'
  began=time.monotonic();os.kill(observed['client'],signal.SIGTERM);output,errors=debug.communicate(timeout=10)
  expected=143 if code is None else code
  assert debug.returncode==0 and exitfile.read_text()==str(expected),(output+errors)[-600:]
  assert out.read_bytes()==(b'' if code is None else b'COMMAND-COMPLETE') and b'owner-timeout' not in err.read_bytes(),(out.read_bytes(),err.read_bytes())
  elapsed=time.monotonic()-began;observed_wait=observed['transport_wait'];os.kill(observed['vmm'],signal.SIGCONT);observed=None
  assert select.select([owner_fd],[],[],60)[0],'owner survived normal teardown'
  run(['stop']);assert not locator(P).exists();control()
  emit(f'leave-signal Linux: '+(f'actual VMM held before EXEC; transport timeout={observed_wait:.2f}s -> Err; CMD=absent' if code is None else f'observed={code}')+f'; Draining before SIGTERM; client={expected}; signal-to-return={elapsed:.2f}s; restart=0/LEAVE-CONTROL')
 finally:
  if observed is not None:
   try:os.kill(observed['vmm'],signal.SIGCONT)
   except ProcessLookupError:pass
  for fd in pipes:os.close(fd)
  if owner_fd is not None:os.close(owner_fd)
  if debug.poll() is None:debug.terminate();debug.communicate(timeout=10)
def mac(code):
 result=None;supervisor=None;master=slave=read=write=None
 if code is None:
  master,slave=os.openpty();read,write=os.pipe();supervisor=os.fork()
  if supervisor==0:
   os.close(read);os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0);os.tcsetpgrp(slave,os.getpgrp());os.close(master)
   child=sp.Popen([B,'--','printf','MUST-NOT-RUN'],cwd=P,env=E,stdin=slave,stdout=slave,stderr=sp.PIPE,process_group=0)
   os.write(write,(str(child.pid)+'\n').encode());errors=child.communicate(timeout=180)[1]
   os.write(write,json.dumps(dict(code=child.returncode,errors=errors.decode())).encode());os._exit(0)
  os.close(write);write=None;stream=os.fdopen(read,'rb',buffering=0);read=None;client=int(stream.readline())
 else:
  child=sp.Popen([B,'--','sh','-c',f'printf READY; read x; sync; exit {code}'],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);client=child.pid
  assert select.select([child.stdout],[],[],90)[0] and child.stdout.read(5)==b'READY'
 queue=select.kqueue();owner=held=None;tracked=set();lib=ctypes.CDLL('/usr/lib/libproc.dylib');paused=set()
 def note(pid):queue.control([select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXIT|select.KQ_NOTE_EXEC)],0,0)
 def resume(pid):
  try:os.kill(pid,signal.SIGCONT)
  except ProcessLookupError:pass
  paused.discard(pid)
 try:
  if code is not None:child.stdin.write(b'G\n');child.stdin.flush()
  deadline=time.monotonic()+90
  while held is None:
   assert time.monotonic()<deadline,'actual removal helper not observed'
   if owner is None:
    ids=(ctypes.c_int*256)();lib.proc_listchildpids(client,ids,ctypes.sizeof(ids))
    for candidate in ids:
     if not candidate:continue
     binary=ctypes.create_string_buffer(4096);lib.proc_pidpath(candidate,binary,len(binary))
     if binary.value!=os.fsencode(B):continue
     args=sp.run(['ps','-p',str(candidate),'-o','command='],capture_output=True).stdout
     if b'__project-owner' in args:owner=candidate;note(owner);break
    if owner is None:continue
   if not locator(P).exists():continue
   observed=loc(P);generation=observed['generation']
   if not generation or not (H/('r-'+generation[:32])/'c.sock').exists():continue
   ids=(ctypes.c_int*256)();lib.proc_listchildpids(owner,ids,ctypes.sizeof(ids));pending={pid for pid in ids if pid and pid not in tracked}
   for pid in pending:
    try:note(pid);tracked.add(pid)
    except ProcessLookupError:pass
   pending.update(e.ident for e in queue.control([],64,0) if e.ident!=owner and e.fflags&select.KQ_NOTE_EXEC)
   for pid in pending:
    try:os.kill(pid,signal.SIGSTOP);paused.add(pid)
    except ProcessLookupError:continue
    binary=ctypes.create_string_buffer(4096);lib.proc_pidpath(pid,binary,len(binary))
    if not binary.value.endswith(b'/cli/container'):resume(pid);continue
    args=sp.run(['ps','-p',str(pid),'-o','command='],capture_output=True).stdout
    if observed['name'].encode() not in args or not any(v in args for v in (b' kill ',b' stop ',b' rm ')):resume(pid);continue
    held=pid;note(pid);break
  deadline=time.monotonic()+2
  while b'T' not in sp.run(['ps','-p',str(held),'-o','state='],capture_output=True,check=True).stdout:assert time.monotonic()<deadline
  began=time.monotonic();os.kill(client,signal.SIGTERM);expected=143 if code is None else code
  if code is None:
   assert select.select([stream],[],[],10)[0],'signalled client did not return';result=json.loads(stream.read());assert result['code']==expected and 'foreground' in result['errors'],result;os.waitpid(supervisor,0);supervisor=None
  else:
   output,errors=child.communicate(timeout=10);assert child.returncode==expected and not output,(child.returncode,output,errors)
  elapsed=time.monotonic()-began;expected_exits={owner,held};resume(held);held=None;exited=set();deadline=time.monotonic()+60
  while exited!=expected_exits:
   assert time.monotonic()<deadline,'owner/helper survived normal teardown'
   for event in queue.control([],64,max(0,deadline-time.monotonic())):
    if event.ident in expected_exits and event.fflags&select.KQ_NOTE_EXIT:exited.add(event.ident)
  run(['stop']);assert not locator(P).exists();control()
  emit('leave-signal Mac: '+('host PTY foreground refusal -> Err; CMD=absent' if code is None else f'observed={code}')+f'; native removal held at SIGTERM; client={expected}; signal-to-return={elapsed:.2f}s; restart=0/LEAVE-CONTROL')
 finally:
  for pid in list(paused):resume(pid)
  queue.close()
  if supervisor is not None:
   parent=sp.run(['ps','-p',str(client),'-o','ppid='],capture_output=True).stdout.strip()
   if parent==str(supervisor).encode():os.kill(client,signal.SIGKILL)
   os.kill(supervisor,signal.SIGTERM);os.waitpid(supervisor,0)
  if code is None:stream.close()
  if code is not None and child.poll() is None:child.terminate();child.communicate(timeout=90)
  for fd in (master,slave,read,write):
   if fd is not None:os.close(fd)
try:
 for code in (None,0,23):(linux if sys.platform=='linux' else mac)(code)
finally:
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup own boxes/disks=True')
