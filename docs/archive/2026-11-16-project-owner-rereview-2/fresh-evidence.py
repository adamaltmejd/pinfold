#!/usr/bin/env python3
"""Linux fresh-claim refusal preserves record, state and live endpoint; BIN IMAGE."""
import hashlib,json,os,pathlib,select,shlex,signal,socket,subprocess as sp,sys,threading
assert sys.platform=='linux'
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
P.mkdir();git(P,'init','-q','-b','proof');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture')
run(['config','edit']);C=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
def case(kind):
 ready=W/(kind+'.ready');go=W/(kind+'.go');status_file=W/(kind+'.status');script=W/(kind+'.gdb');out=W/(kind+'.out');err=W/(kind+'.err');debuglog=W/(kind+'.debug')
 for path in (ready,go):os.mkfifo(path)
 read=os.open(ready,os.O_RDWR|os.O_NONBLOCK);keeper=os.open(go,os.O_RDWR|os.O_NONBLOCK|os.O_CLOEXEC)
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,threading
starter=None
status_thread=None
def status():
 pid,value=os.waitpid(starter,0)
 pathlib.Path({str(status_file)!r}).write_text(str(os.waitstatus_to_exitcode(value)))
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:self.enabled=False;gdb.execute('set follow-fork-mode parent')
  return False
class Gate(gdb.Breakpoint):
 def stop(self):
  global status_thread
  owner=gdb.selected_inferior().pid
  if starter is None or owner==starter:return False
  self.enabled=False
  value={{'owner':owner,'starter':starter,'locator':json.loads(pathlib.Path({str(locator(P))!r}).read_text())}}
  with open({str(ready)!r},'w') as stream:stream.write(json.dumps(value)+'\\n');stream.flush()
  status_thread=threading.Thread(target=status);status_thread.start()
  with open({str(go)!r},'rb') as stream:assert stream.read(1)==b'G'
  return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Gate('pinfold::core::ownership::Claim::prepare_fresh')
end
run -- printf MUST-NOT-RUN > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
if status_thread is not None:
 status_thread.join(90)
 assert not status_thread.is_alive(),'starter did not exit'
end
''')
 with debuglog.open('wb') as log:debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=log,stderr=sp.STDOUT)
 observed=None;owner_fd=starter_fd=None;listener=None;record=state=endpoint=None;state_inode=socket_inode=None;saved={}
 def remember(path):saved[path]=(path.lstat().st_ino,path.read_bytes())
 def unchanged():
  for path,(inode,data) in saved.items():assert path.lstat().st_ino==inode and path.read_bytes()==data,'seeded evidence changed'
 def exchange():
  with socket.socket(socket.AF_UNIX) as probe:
   probe.settimeout(2);probe.connect(str(endpoint));accepted,_=listener.accept()
   with accepted:
    probe.sendall(b'PING');assert accepted.recv(4)==b'PING';accepted.sendall(b'PONG');assert probe.recv(4)==b'PONG'
 try:
  assert select.select([read],[],[],90)[0],'fresh-preparation debugger barrier absent'
  observed=json.loads(os.read(read,65536));owner_fd=os.pidfd_open(observed['owner']);starter_fd=os.pidfd_open(observed['starter']);name=observed['locator']['name'];assert observed['locator']['generation'] is None
  key=hashlib.sha256(('box:'+name).encode()).hexdigest();record=H/(key+'.owner');endpoint=H/(key+'.sock');state=W/'s/pinfold/boxes'/hashlib.sha256(name.encode()).hexdigest()[:16]
  assert not record.exists() and not endpoint.exists() and not state.exists()
  if kind in ('record','state'):
   state.mkdir(parents=True);marker=state/'marker';marker.write_bytes(b'PRESERVE');remember(marker);state_inode=state.lstat().st_ino
   if kind=='record':
    vmm=state/'vmm';vmm.write_bytes(b'not-started\n');remember(vmm)
    payload=json.dumps(dict(name=name,generation=hashlib.sha256(b'fixture-generation').hexdigest(),retiring=False,state_dir=str(state),disks=[],described=None)).encode()
    fd=os.open(record,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600);os.write(fd,payload);os.close(fd);remember(record);assert record.stat().st_mode&0o777==0o600
  else:
   listener=socket.socket(socket.AF_UNIX);listener.bind(str(endpoint));endpoint.chmod(0o600);listener.listen(4);listener.settimeout(2);socket_inode=endpoint.lstat().st_ino;exchange()
  os.write(keeper,b'G');assert select.select([owner_fd],[],[],90)[0],'owner did not refuse pre-existing evidence'
  debug.communicate(timeout=100);assert debug.returncode==0,debuglog.read_bytes()[-1200:]
  assert status_file.read_text()=='125' and out.read_bytes()==b'' and b'owner-unavailable' in err.read_bytes(),(status_file.read_text(),out.read_bytes(),err.read_bytes())
  unchanged()
  if kind=='record':assert record.stat().st_mode&0o777==0o600
  if kind in ('record','state'):assert state.lstat().st_ino==state_inode
  else:assert endpoint.lstat().st_ino==socket_inode;exchange()
  # Remove only the exact fixture bytes/inodes before asking checked recovery to proceed.
  if listener is not None:listener.close();listener=None;assert endpoint.lstat().st_ino==socket_inode;endpoint.unlink()
  for path,(inode,data) in saved.items():assert path.lstat().st_ino==inode and path.read_bytes()==data;path.unlink()
  saved.clear()
  if kind in ('record','state'):assert state.lstat().st_ino==state_inode;state.rmdir()
  run(['stop']);assert run(['--','printf','FRESH-CONTROL']).stdout==b'FRESH-CONTROL';run(['stop'])
  emit(f'fresh-{kind}: required prepare_fresh barrier; launcher=125/owner-unavailable CMD=absent; seeded bytes/inodes preserved'+('; original socket still answered PONG' if kind=='socket' else '')+'; exact seed removed; checked restart=0/FRESH-CONTROL')
 finally:
  os.write(keeper,b'G')
  if starter_fd is not None and not select.select([starter_fd],[],[],0)[0]:signal.pidfd_send_signal(starter_fd,signal.SIGTERM)
  if debug.poll() is None:
   debug.terminate()
   try:debug.communicate(timeout=10)
   except sp.TimeoutExpired:debug.kill();debug.communicate(timeout=10)
  if listener is not None:listener.close()
  for path,(inode,data) in list(saved.items()):
   if path.exists() and path.lstat().st_ino==inode and path.read_bytes()==data:path.unlink()
  if state_inode is not None and state is not None and state.exists() and state.lstat().st_ino==state_inode:
   if not list(state.iterdir()):state.rmdir()
  if socket_inode is not None and endpoint is not None and endpoint.exists() and endpoint.lstat().st_ino==socket_inode:endpoint.unlink()
  for fd in (read,keeper,owner_fd,starter_fd):
   if fd is not None:os.close(fd)
  run(['stop'])
try:
 for kind in ('record','state','socket'):case(kind)
finally:
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup exact own fixtures/boxes/disks=True')
