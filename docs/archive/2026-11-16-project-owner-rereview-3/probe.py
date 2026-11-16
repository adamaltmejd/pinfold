#!/usr/bin/env python3
"""Linux actual fork-only namespace probe: BIN IMAGE. Root debugger, ordinary-uid binary."""
import hashlib,json,os,pathlib,select,shlex,signal,subprocess as sp,sys,threading,time
assert sys.platform=='linux' and os.getuid()!=0
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
P.mkdir();git(P,'init','-q','-b','probe');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture')
run(['config','edit']);C=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
ready=W/'probe.ready';os.mkfifo(ready);read=os.open(ready,os.O_RDWR|os.O_NONBLOCK)
script=W/'probe.gdb';debuglog=W/'probe.debug';out=W/'probe.out';err=W/'probe.err';status_file=W/'probe.status'
script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
set exec-wrapper /usr/bin/setpriv --reuid={os.getuid()} --regid={os.getgid()} --clear-groups
handle SIGPIPE nostop noprint pass
python
import gdb,hashlib,json,os,pathlib,signal,threading
starter=None
owner=None
owner_number=None
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  if starter is not None and gdb.selected_inferior().pid!=starter:self.enabled=False;gdb.execute('set follow-fork-mode parent')
  return False
class Probe(gdb.Breakpoint):
 def stop(self):
  global owner,owner_number
  owner=gdb.selected_inferior().pid;owner_number=gdb.selected_inferior().num
  assert owner!=starter
  self.enabled=False;gdb.execute('set detach-on-fork off');gdb.execute('set follow-fork-mode child');return False
class Isolated(gdb.Breakpoint):
 def stop(self):
  if owner is None or gdb.selected_inferior().pid==owner:return False
  self.enabled=False;return True
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Probe('pinfold::core::runtime::firecracker::jail::probe');Isolated('pinfold::core::runtime::firecracker::jail::enter')
end
run -- printf MUST-NOT-RUN > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
probe=gdb.selected_inferior().pid;assert owner is not None and probe!=owner,'required isolated probe breakpoint absent'
os.kill(probe,signal.SIGSTOP)
gdb.execute('detach')
gdb.execute('inferior '+str(owner_number));gdb.execute('set follow-fork-mode parent');gdb.execute('set detach-on-fork on')
observed=json.loads(pathlib.Path({str(locator(P))!r}).read_text());claim=json.loads((pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')).read_text())
with open({str(ready)!r},'w') as f:f.write(json.dumps(dict(owner=owner,starter=starter,probe=probe,name=observed['name'],generation=claim['generation']))+'\\n');f.flush()
def status():
 pid,value=os.waitpid(starter,0);pathlib.Path({str(status_file)!r}).write_text(str(os.waitstatus_to_exitcode(value)))
status_thread=threading.Thread(target=status);status_thread.start()
end
continue
python
status_thread.join(90);assert not status_thread.is_alive(),'starter did not exit'
end
''')
with debuglog.open('wb') as log:
 debug=sp.Popen(['sudo','-E','gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=log,stderr=sp.STDOUT)
observed=None;handles=[]
try:
 assert select.select([read],[],[],90)[0],'required fork-only probe barrier absent: '+debuglog.read_text()[-1200:]
 observed=json.loads(os.read(read,65536));handles=[os.pidfd_open(observed[k]) for k in ('owner','starter','probe')]
 deadline=time.monotonic()+2
 while not pathlib.Path('/proc',str(observed['probe']),'status').read_text().split('State:',1)[1].lstrip().startswith('T'):
  assert time.monotonic()<deadline,'probe did not stop'
 assert pathlib.Path('/proc',str(observed['owner']),'status').read_text().split('Uid:',1)[1].split()[:4]==[str(os.getuid())]*4,'binary ran privileged'
 listing=sp.run(['sudo','python3','-c','import json,os,pathlib,sys;p=pathlib.Path("/proc")/sys.argv[1]/"fd";print(json.dumps({x.name:os.readlink(x) for x in p.iterdir()}))',str(observed['probe'])],capture_output=True,check=True)
 descriptors=json.loads(listing.stdout);assert set(descriptors)==set('01234'),descriptors
 assert all(descriptors[str(n)]=='/dev/null' for n in range(3)),descriptors
 assert all(descriptors[str(n)].startswith('pipe:[') for n in (3,4)) and descriptors['3']!=descriptors['4'],descriptors
 emit('probe descriptor listing: '+json.dumps(descriptors,sort_keys=True)+'; no lock/lease/proxy/control descriptors')
 began=time.monotonic();run(['stop']);elapsed=time.monotonic()-began
 assert all(select.select([handle],[],[],30)[0] for handle in handles),'probe/owner/starter survived stop'
 debug.communicate(timeout=100);assert debug.returncode==0,debuglog.read_bytes()[-1200:]
 assert status_file.read_text()=='125' and not out.read_bytes() and b'stopped' in err.read_bytes(),(status_file.read_text(),out.read_bytes(),err.read_bytes())
 assert elapsed<10 and not locator(P).exists() and not (H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')).exists()
 assert run(['--','printf','PROBE-CONTROL']).stdout==b'PROBE-CONTROL';run(['stop'])
 emit(f'probe cancellation: actual namespace child SIGSTOP; stop=0 launcher=125/stopped CMD=absent; child/owner kernel exit; seconds={elapsed:.2f}; next start=0/PROBE-CONTROL')
finally:
 if observed is not None:
  for handle in handles:
   if not select.select([handle],[],[],0)[0]:
    try:signal.pidfd_send_signal(handle,signal.SIGKILL)
    except ProcessLookupError:pass
 if debug.poll() is None:
  sp.run(['sudo','kill','-TERM',str(debug.pid)],check=False)
  try:debug.communicate(timeout=10)
  except sp.TimeoutExpired:sp.run(['sudo','kill','-KILL',str(debug.pid)],check=False);debug.communicate(timeout=10)
 for fd in [read,*handles]:os.close(fd)
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup exact own boxes/disks=True; debugger privileged, binary ordinary uid')
