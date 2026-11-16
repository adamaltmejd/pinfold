#!/usr/bin/env python3
"""Real completed-exec/stop barriers: status-race.py BIN IMAGE; no sleeps."""
import ctypes,hashlib,json,os,pathlib,select,shlex,signal,subprocess as sp,sys,tempfile
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];R=pathlib.Path(__file__).resolve().parents[3]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfos.',dir='/tmp')).resolve();P=W/'p';P.mkdir();H=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
def emit(s):print(str(s).replace(str(W),'$W').replace(B,'$BIN').replace(str(R),'$REPO').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,code=0):
 r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=100);assert r.returncode==code,(args,r.returncode,r.stderr[:300]);return r
sp.run(['git','init','-q',str(P)],check=True)
sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
C=W/'c/pinfold/projects'/ID/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
L=H/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project')
def linux(code):
 source=R/'crates/pinfold/src/core/runtime/firecracker/vsock.rs';lines=source.read_text().splitlines()
 points=[i+1 for i,x in enumerate(lines) if 'return Ok(i32::from_be_bytes(code));' in x];assert len(points)==1,'EXIT anchor absent/ambiguous'
 evidence=W/f'exit-{code}.json';commands=W/f'exit-{code}.gdb';out=W/f'exit-{code}.out';err=W/f'exit-{code}.err'
 commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,pathlib,subprocess
class Completed(gdb.Breakpoint):
 def stop(self):
  raw=bytes(int(gdb.parse_and_eval('code[%d]'%i)) for i in range(4))
  observed=int.from_bytes(raw,'big',signed=True)
  assert observed=={code},'breakpoint did not observe fixture EXIT'
  self.enabled=False
  stopped=subprocess.run([{B!r},'stop'],cwd={str(P)!r},input=b'',capture_output=True,timeout=20)
  assert stopped.returncode==0,'targeted stop failed at held EXIT'
  pathlib.Path({str(evidence)!r}).write_text(json.dumps({{'observed':observed,'stop':stopped.returncode}}))
  return False
Completed({str(source)+':'+str(points[0])!r})
end
run -- sh -c {shlex.quote('exit '+str(code))} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
p=pathlib.Path({str(evidence)!r})
assert p.exists(),'required EXIT breakpoint was not hit'
r=json.loads(p.read_text());r['client']=int(gdb.parse_and_eval('$_exitcode'));p.write_text(json.dumps(r))
end
''')
 result=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',B],cwd=P,env=E,capture_output=True,timeout=150)
 assert result.returncode==0 and evidence.exists(),('required GDB EXIT/stop barrier failed',(result.stdout+result.stderr)[-900:])
 actual=json.loads(evidence.read_text());assert actual=={'observed':code,'stop':0,'client':code},actual
 assert not L.exists(),'stop left locator'
 emit(f'linux expected={code} decoded-EXIT={actual["observed"]} debugger-held-client=True stop=0 client={actual["client"]}')
def native_pid(parent):
 library=ctypes.CDLL('/usr/lib/libproc.dylib');pids=(ctypes.c_int*128)();count=library.proc_listchildpids(parent,pids,ctypes.sizeof(pids))
 for pid in pids[:count]:
  if pid:
   r=sp.run(['ps','-p',str(pid),'-o','args='],capture_output=True)
   if r.returncode==0 and b' exec -i' in r.stdout:return pid
 raise AssertionError('native exec child absent at guest FIFO barrier')
def apple(code):
 read,write=os.pipe();os.set_blocking(write,False)
 try:
  while True:os.write(write,b'F'*4096)
 except BlockingIOError:pass
 os.set_blocking(write,True);gate=f'/tmp/probe-status-{code}'
 script=f'mkfifo {gate}; echo EXIT-BARRIER >&2; cat {gate} >/dev/null; printf DONE; exit {code}'
 child=sp.Popen([B,'--','sh','-c',script],cwd=P,env=E,stdin=sp.DEVNULL,stdout=write,stderr=sp.PIPE);os.close(write);queue=None;held=False
 try:
  seen=b''
  while b'EXIT-BARRIER\n' not in seen:
   assert select.select([child.stderr],[],[],90)[0],'guest FIFO barrier absent'
   part=os.read(child.stderr.fileno(),4096);assert part,'client ended before guest barrier';seen+=part
  native=native_pid(child.pid);queue=select.kqueue();queue.control([select.kevent(native,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ONESHOT,fflags=select.KQ_NOTE_EXIT)],0,0)
  observed=json.loads(L.read_text());run(['box','exec',observed['name'],'--','sh','-c',f'printf release >{gate}'])
  assert queue.control(None,1,30),'native exit not observed'
  os.kill(child.pid,signal.SIGSTOP);pid,status=os.waitpid(child.pid,os.WUNTRACED);assert pid==child.pid and os.WIFSTOPPED(status),'client escaped hold';held=True
  run(['stop']);assert not L.exists();os.kill(child.pid,signal.SIGCONT);held=False
  actual=child.wait(timeout=30);assert actual==code,(code,actual)
  emit(f'apple expected={code} native-exit=kqueue stdout=full SIGSTOP=confirmed stop=0 client={actual}')
 finally:
  if queue:queue.close()
  if held:os.kill(child.pid,signal.SIGCONT)
  if child.poll() is None:child.terminate();child.wait(timeout=30)
  child.stderr.close();os.close(read)
def stale_stop():
 evidence=W/'stale.json';commands=W/'stale.gdb';run(['start'])
 commands.write_text(f"""set pagination off
set confirm off
set print thread-events off
python
import gdb,json,pathlib,subprocess
class Held(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  old=json.loads(pathlib.Path({str(L)!r}).read_text())
  for argv in ([{B!r},'box','down',old['name']],[{B!r},'start']):
   p=subprocess.run(argv,cwd={str(P)!r},input=b'',capture_output=True,timeout=30)
   assert p.returncode==0,'replacement setup failed'
  new=json.loads(pathlib.Path({str(L)!r}).read_text());assert old['generation']!=new['generation'] and old['name']!=new['name']
  pathlib.Path({str(evidence)!r}).write_text(json.dumps(new))
  return False
Held('pinfold::pi::session::stop_observed')
end
run stop
python
assert pathlib.Path({str(evidence)!r}).exists(),'required stale-stop barrier absent'
assert int(gdb.parse_and_eval('$_exitcode'))==0,'stale stop did not complete'
end
""")
 result=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',B],cwd=P,env=E,capture_output=True,timeout=150)
 assert result.returncode==0 and evidence.exists(),('stale-stop GDB failed',(result.stdout+result.stderr)[-700:])
 expected=json.loads(evidence.read_text());assert json.loads(L.read_text())==expected
 assert run(['--','sh','-c','printf STILL-ALIVE']).stdout==b'STILL-ALIVE';run(['stop'])
 emit('linux stale-stop: observed generation held across teardown/replacement; old stop=0 replacement marker=STILL-ALIVE fresh stop=0')
try:
 for code in (0,23,125,126,127):
  run(['start']);(linux if sys.platform=='linux' else apple)(code)
 if sys.platform=='linux':stale_stop()
 else:emit('NOT-RUN Apple stale-stop instruction hold: Linux debugger case covers shared controller')
 emit('PASS completed exec precedes successful owner shutdown for all five fixture statuses')
 emit('NOT-RUN this driver: command completion versus audit failure or actual teardown failure; use lifecycle cases for cleanup-status controls')
finally:
 try:run(['stop'])
 except Exception as error:emit('cleanup-stop-failed='+str(error))
 try:run(['disk','rm','--all'])
 except Exception as error:emit('cleanup-disk-failed='+str(error))
 emit('fixture='+str(W))
