#!/usr/bin/env python3
"""Late audit failure through the real proxy: BIN IMAGE [clean|leave|removal|all]."""
import ctypes,http.server,queue,shlex,signal,socket,struct,threading,time
import json,os,pathlib,select,subprocess as sp,sys
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
ONLY=sys.argv[3] if len(sys.argv)>3 else 'all';R=pathlib.Path(__file__).resolve().parents[3]
P.mkdir();git(P,'init','-q','-b','review');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture');run(['config','edit'])
receipt=threading.Event();release=threading.Event()
class Fixture(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  waiting,resumed=receipt,release
  if self.path=='/hold':waiting.set();assert resumed.wait(180),'fixture release missing'
  self.send_response(200);self.end_headers()
  try:self.wfile.write(b'AUDIT-CONTROL\n')
  except (BrokenPipeError,ConnectionResetError):pass
 def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Fixture);server_thread=threading.Thread(target=server.serve_forever);server_thread.start()
c=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\nroutes={{audit="127.0.0.1:{server.server_port}"}}\n')
def line(stream):
 assert select.select([stream],[],[],90)[0],'guest output barrier missing'
 return stream.readline()
def owner_info(observed):
 record=H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner');claim=json.loads(record.read_text())
 boxes=[json.loads(x) for x in run(['box','list','--label','dev.pinfold.owner']).stdout.splitlines()]
 owner=next(int(x['owner']) for x in boxes if x['name']==observed['name'] and x['labels']['dev.pinfold.generation']==observed['generation'])
 return owner,claim
def draining(observed):
 deadline=time.monotonic()+10
 while True:
  stream=socket.socket(socket.AF_UNIX);stream.settimeout(1);stream.connect(str(H/('r-'+observed['generation'][:32])/'c.sock'))
  payload=json.dumps(dict(version=1,request=dict(verb='probe',root=str(P),generation=observed['generation'],roots=dict(config='',state='',cache='',home='',codex_home=None)))).encode();stream.sendall(struct.pack('!I',len(payload))+payload)
  def read(size):
   data=b''
   while len(data)<size:
    part=stream.recv(size-len(data));assert part;data+=part
   return data
  answer=json.loads(read(struct.unpack('!I',read(4))[0]))['reply'];stream.close()
  if answer==dict(verb='draining',generation=observed['generation']):return
  assert time.monotonic()<deadline,'owner never entered draining'
def native_removal(owner,name,done,found,paused):
 lib=ctypes.CDLL('/usr/lib/libproc.dylib');events=select.kqueue();tracked=set()
 try:
  deadline=time.monotonic()+90
  while not done.is_set():
   assert time.monotonic()<deadline,'native removal helper missing'
   ids=(ctypes.c_int*256)();lib.proc_listchildpids(owner,ids,ctypes.sizeof(ids));pending=set(pid for pid in ids if pid and pid not in tracked)
   for pid in pending:
    try:events.control([select.kevent(pid,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ENABLE,fflags=select.KQ_NOTE_EXEC|select.KQ_NOTE_EXIT)],0,0);tracked.add(pid)
    except ProcessLookupError:pass
   pending.update(event.ident for event in events.control([],64,0) if event.fflags&select.KQ_NOTE_EXEC)
   for pid in pending:
    try:os.kill(pid,signal.SIGSTOP);paused.add(pid)
    except ProcessLookupError:continue
    path=ctypes.create_string_buffer(4096);lib.proc_pidpath(pid,path,len(path))
    args=sp.run(['ps','-p',str(pid),'-o','ppid=,command='],capture_output=True,timeout=5).stdout.decode().strip().split(None,1)
    if path.value.endswith(b'/cli/container') and len(args)==2 and int(args[0])==owner and name in args[1] and any(v in args[1] for v in (' kill ',' stop ',' rm ')):
     stopping=time.monotonic()+2
     while True:
      state=sp.run(['ps','-p',str(pid),'-o','state='],capture_output=True,check=True,timeout=5).stdout
      if b'T' in state:break
      assert time.monotonic()<stopping,'native helper did not stop'
     found.put(pid);return
    try:os.kill(pid,signal.SIGCONT);paused.discard(pid)
    except ProcessLookupError:paused.discard(pid)
 except BaseException as error:found.put(error)
 finally:events.close()
def debugger(code,command,stdin):
 ready=W/f'audit-{code}.ready';os.mkfifo(ready);fd=os.open(ready,os.O_RDWR|os.O_NONBLOCK)
 output,errors,status,script=[W/(f'audit-{code}'+suffix) for suffix in ('.out','.err','.exit','.gdb')]
 source=R/'crates/pinfold/src/pi/session.rs';anchors=[i+1 for i,s in enumerate(source.read_text().splitlines()) if 'let leave = lease.leave(signals).await;' in s];assert len(anchors)==1
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,hashlib,json,os,pathlib,signal
class Completed(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  observed=json.loads(pathlib.Path({str(locator(P))!r}).read_text());claim=json.loads((pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')).read_text());vmm=int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]);os.kill(vmm,signal.SIGSTOP)
  with open({str(ready)!r},'w') as f:f.write(str(vmm)+'\\n');f.flush()
  return False
Completed({str(source)+':'+str(anchors[0])!r})
gdb.events.exited.connect(lambda event:pathlib.Path({str(status)!r}).write_text(str(event.exit_code)))
end
run -- sh -c {shlex.quote(command)} < {shlex.quote(str(stdin))} > {shlex.quote(str(output))} 2> {shlex.quote(str(errors))}
''')
 process=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
 return process,fd,output,errors,status

def case(mode,code):
 global receipt,release
 receipt,release=threading.Event(),threading.Event();child=debug=watcher=None;done=threading.Event();found=queue.Queue();paused=set();held_vm=None;vm_handle=None;ready=None
 input_path=W/f'{mode}-{code}.stdin';os.mkfifo(input_path);input_fd=os.open(input_path,os.O_RDWR|os.O_NONBLOCK)
 command=f'set -e; curl -fsS http://audit/control; curl -fsS http://audit/hold </dev/null >/dev/null 2>&1 & echo READY; read x; sync; exit {code}'
 observed=None;log=saved=None
 try:
  if mode=='removal' and sys.platform=='linux':
   debug,ready,output,errors,status=debugger(code,command,input_path)
  else:
   reader=os.open(input_path,os.O_RDONLY)
   try:child=sp.Popen([B,'--','sh','-c',command],cwd=P,env=E,stdin=reader,stdout=sp.PIPE,stderr=sp.PIPE,bufsize=0)
   finally:os.close(reader)
   assert line(child.stdout)==b'AUDIT-CONTROL\n' and line(child.stdout)==b'READY\n'
  assert receipt.wait(90),'pending real route request missing';observed=loc(P);generation=observed['generation'];owner,claim=owner_info(observed)
  log=W/'s/pinfold/runs'/generation/'egress.jsonl';saved=log.with_suffix('.saved');assert log.is_file()
  if mode=='removal' and sys.platform=='darwin':watcher=threading.Thread(target=native_removal,args=(owner,observed['name'],done,found,paused));watcher.start()
  if mode=='leave':log.rename(saved);log.mkdir(mode=0o700)
  began=time.monotonic();os.write(input_fd,b'go\n')
  if mode=='removal':
   if sys.platform=='linux':
    assert select.select([ready],[],[],90)[0],'observed guest EXIT debugger barrier missing';held_vm=int(os.read(ready,128));vm_handle=os.pidfd_open(held_vm);assert pathlib.Path(f'/proc/{held_vm}/status').read_text().split('State:',1)[1].lstrip().startswith('T')
   else:
    helper=found.get(timeout=90);assert isinstance(helper,int),helper
   draining(observed);log.rename(saved);log.mkdir(mode=0o700)
   if sys.platform=='darwin':os.kill(helper,signal.SIGCONT);paused.discard(helper)
  if debug is not None:
   out,err=debug.communicate(timeout=150);assert debug.returncode==0,(out+err)[-800:];actual=int(status.read_text());out=output.read_bytes();err=errors.read_bytes();assert out==b'AUDIT-CONTROL\nREADY\n'
  else:out,err=child.communicate(timeout=150);actual=child.returncode;assert not out
  elapsed=time.monotonic()-began;assert actual==code,(actual,err)
  terminal=json.loads((W/'s/pinfold/runs'/generation/'terminal.json').read_text());expected='sessions-ended' if mode=='clean' else 'audit-log'
  assert terminal['reason']==expected and terminal['detail'] is None and not locator(P).exists(),terminal
  if mode=='clean':assert b'audit-log' not in err,err
  else:assert b'audit-log' in err and generation.encode() in err and b'pinfold box inspect' in err,err
  emit(f'audit-{mode}: pending real route; observed={code} client={actual} reason={expected} terminal={expected}/null generation='+('notified' if mode!='clean' else 'clean')+f' seconds={elapsed:.2f}; same-box allowed HTTP=AUDIT-CONTROL; locator=absent')
 finally:
  release.set();done.set()
  for pid in list(paused):
   try:os.kill(pid,signal.SIGCONT)
   except ProcessLookupError:pass
  if watcher is not None:watcher.join(timeout=10);assert not watcher.is_alive()
  if held_vm is not None and vm_handle is not None:
   if not select.select([vm_handle],[],[],0)[0]:signal.pidfd_send_signal(vm_handle,signal.SIGCONT)
   os.close(vm_handle)
  if log is not None and log.is_dir():log.rmdir()
  if saved is not None and saved.exists():saved.rename(log)
  os.close(input_fd)
  if ready is not None:os.close(ready)
  run(['stop'])
  for process in (child,debug):
   if process is not None and process.poll() is None:process.terminate();process.communicate(timeout=90)
 assert run(['--','printf','AUDIT-RESTART']).stdout==b'AUDIT-RESTART'
 emit(f'audit-{mode} control: repaired own log; next start=0/AUDIT-RESTART; normal departure')
try:
 for mode in ('clean','leave','removal'):
  if ONLY in ('all',mode):
   for code in (0,23):case(mode,code)
finally:
 release.set();run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 server.shutdown();server.server_close();server_thread.join();emit('cleanup own boxes/disks=True')
