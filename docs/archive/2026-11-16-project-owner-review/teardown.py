#!/usr/bin/env python3
"""Real slow last departure/draining/failure: teardown.py BIN IMAGE (Apple or Linux)."""
import json,os,pathlib,queue,select,shlex,signal,socket,struct,subprocess as sp,sys,threading,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner/crashes.py'
exec(compile(old.read_text().split('def crash(')[0],str(old),'exec'),globals())
SESSION=R/'crates/pinfold/src/pi/session.rs'
def source_line(text):
 lines=SESSION.read_text().splitlines();found=[i+1 for i,line in enumerate(lines) if text in line];assert len(found)==1,(text,found);return str(SESSION)+':'+str(found[0])
def control():
 assert run(['--','sh','-c','printf TEARDOWN-CONTROL']).stdout==b'TEARDOWN-CONTROL';run(['stop']);assert not L.exists()
def frame(request):
 data=json.dumps(dict(version=1,request=request)).encode();return struct.pack('!I',len(data))+data
def reply(stream):
 def read(size):
  data=b''
  while len(data)<size:
   part=stream.recv(size-len(data));assert part,'reply ended early';data+=part
  return data
 return json.loads(read(struct.unpack('!I',read(4))[0]))['reply']
def slow(code):
 ready=W/f'leave-{code}.ready';go=W/f'leave-{code}.go';ack=W/f'leave-{code}.ack'
 for path in (ready,go,ack):os.mkfifo(path)
 pipes=[os.open(path,os.O_RDWR|os.O_NONBLOCK) for path in (ready,ack)]
 script=W/f'leave-{code}.gdb';out=W/f'leave-{code}.out';err=W/f'leave-{code}.err';exitfile=W/f'leave-{code}.exit'
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,signal,hashlib
class Completed(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  loc=json.loads(pathlib.Path({str(L)!r}).read_text());record=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');claim=json.loads(record.read_text());vmm=int((pathlib.Path(claim['state_dir'])/'vmm').read_text().split()[0]);os.kill(vmm,signal.SIGSTOP)
  with open({str(ready)!r},'w') as f:f.write(json.dumps(loc)+'\\n');f.flush()
  with open({str(go)!r},'rb') as f:assert f.read(1)==b'G'
  return False
class Draining(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  with open({str(ack)!r},'w') as f:f.write('D');f.flush()
  return False
Completed({source_line('let leave = lease.leave(signals).await;')!r})
Draining({source_line('acknowledged = true;')!r})
def exited(event):pathlib.Path({str(exitfile)!r}).write_text(str(event.exit_code))
gdb.events.exited.connect(exited)
end
run -- sh -c 'printf COMMAND-COMPLETE; sync; exit {code}' > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
''')
 debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdout=sp.PIPE,stderr=sp.PIPE);connections=[];before=time.monotonic()
 try:
  assert select.select([pipes[0]],[],[],100)[0],'completed-exec gate absent';loc=json.loads(os.read(pipes[0],4096));generation=loc['generation'];before=time.monotonic();endpoint=H/('r-'+generation[:32])/'c.sock'
  roots=dict(config='',state='',cache='',home='',codex_home=None)
  snapshot=dict(roots=roots,repository=True,profile=dict(name='default',built_in=True),image=dict(source='image',reference=IMAGE,id=None),resources=dict(cpus=2.,memory_mib=1024),mounts=[],disks=[],allow=[],routes={},route_values={},env_names={},env_values={})
  for verb in ('probe','join','keep','borrow'):
   request=dict(verb=verb,root=str(P),generation=generation);request['snapshot' if verb in ('join','keep') else 'roots']=snapshot if verb in ('join','keep') else roots
   data=frame(request);stream=socket.socket(socket.AF_UNIX);stream.settimeout(1);stream.connect(str(endpoint));stream.sendall(data[:5]);connections.append((verb,stream,data[5:]))
  with go.open('wb',buffering=0) as f:f.write(b'G')
  assert select.select([pipes[1]],[],[],10)[0] and os.read(pipes[1],1)==b'D','last Leave did not receive Draining acknowledgement'
  for verb,stream,data in connections:
   began=time.monotonic();stream.sendall(data);answer=reply(stream);elapsed=time.monotonic()-began;assert answer==dict(verb='draining',generation=generation) and elapsed<=1,(verb,answer,elapsed)
   emit(f'completed={code} in-flight-{verb}: reply=Draining same-generation=True seconds={elapsed:.3f}')
  output,errors=debug.communicate(timeout=100);elapsed=time.monotonic()-before;assert debug.returncode==0 and exitfile.read_text()==str(code),(output+errors)[-600:]
  assert out.read_bytes()==b'COMMAND-COMPLETE' and b'owner-timeout' not in err.read_bytes() and elapsed>10,(out.read_bytes(),err.read_bytes(),elapsed)
  terminal=json.loads((W/'s/pinfold/runs'/generation/'terminal.json').read_text());assert terminal['reason']=='sessions-ended' and terminal['detail'] is None and not L.exists()
  emit(f'slow-last-leave observed={code} client={code} seconds={elapsed:.2f} terminal=sessions-ended/null timeout-diagnostic=absent locator=absent');control()
 finally:
  for _,stream,_ in connections:stream.close()
  for pipe in pipes:os.close(pipe)
  if debug.poll() is None:debug.terminate();debug.communicate(timeout=10)

def macslow(code):
 child=process(f'echo READY; read x; sync; exit {code}');loc=barrier(child)
 listed=[json.loads(line) for line in run(['box','list','--label','dev.pinfold.owner']).stdout.splitlines()]
 owner=next(int(box['owner']) for box in listed if box['name']==loc['name'] and box['labels']['dev.pinfold.generation']==loc['generation'])
 found=queue.Queue();done=threading.Event();held=None;paused=[]
 def observe():
  try:
   deadline=time.monotonic()+100
   while not done.is_set():
    assert time.monotonic()<deadline and child.poll() is None,'native removal helper barrier missed'
    rows=sp.run(['ps','-axo','pid=,ppid='],capture_output=True,check=True).stdout.splitlines()
    for row in rows:
     pid,ppid=map(int,row.split())
     if ppid!=owner:continue
     command=sp.run(['ps','-p',str(pid),'-o','command='],capture_output=True).stdout
     if loc['name'].encode() in command and any(verb in command for verb in (b' kill ',b' stop ',b' rm ')):
      os.kill(pid,signal.SIGSTOP);paused.append(pid);state=sp.run(['ps','-p',str(pid),'-o','state='],capture_output=True,check=True).stdout;assert b'T' in state,'native removal helper exited before hold'
      found.put(pid);return
  except BaseException as error:found.put(error)
 watcher=threading.Thread(target=observe);watcher.start();began=time.monotonic()
 try:
  child.stdin.write(b'go\n');child.stdin.flush();held=found.get(timeout=100);assert isinstance(held,int),held
  assert not select.select([child.stdout],[],[],12)[0] and child.poll() is None,'last client returned before held removal completed'
  stream=socket.socket(socket.AF_UNIX);stream.settimeout(1);stream.connect(str(H/('r-'+loc['generation'][:32])/'c.sock'))
  started=time.monotonic();stream.sendall(frame(dict(verb='probe',root=str(P),generation=loc['generation'],roots=dict(config='',state='',cache='',home='',codex_home=None))));answer=reply(stream);latency=time.monotonic()-started;stream.close()
  assert answer==dict(verb='draining',generation=loc['generation']) and latency<=1,(answer,latency)
  os.kill(held,signal.SIGCONT);paused.clear();held=None;child.wait(timeout=100);out,err=child.communicate();assert child.returncode==code and not out and b'owner-timeout' not in err,(child.returncode,out,err)
  terminal=json.loads((W/'s/pinfold/runs'/loc['generation']/'terminal.json').read_text());assert terminal['reason']=='sessions-ended' and terminal['detail'] is None and not L.exists()
  emit(f'apple-slow-last-leave observed={code} client={code} seconds={time.monotonic()-began:.2f} held-native=SIGSTOP-confirmed pending-after-12s=True Probe=Draining/{latency:.3f}s terminal=sessions-ended/null');control()
 finally:
  done.set();watcher.join(timeout=5);assert not watcher.is_alive()
  for pid in paused:
   parent=sp.run(['ps','-p',str(pid),'-o','ppid='],capture_output=True).stdout.strip()
   if parent==str(owner).encode():os.kill(pid,signal.SIGCONT)
  if child.poll() is None:child.terminate();child.communicate(timeout=100)

def process(command):return sp.Popen([B,'--','sh','-c',command],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
def barrier(child):
 assert select.select([child.stdout],[],[],100)[0] and child.stdout.readline()==b'READY\n','guest command barrier absent';return json.loads(L.read_text())
def stopped():
 child=process('echo READY; read x');loc=barrier(child);run(['stop']);child.wait(timeout=100);out,err=child.communicate();assert child.returncode==125 and not out and b'signal' in err and loc['generation'].encode() in err and b'pinfold box inspect' in err,(child.returncode,out,err)
 emit('external-stop: incomplete-client=125 reason=signal generation=present inspection-command=present');control()
def cleanup_failure(code):
 child=process(f'sync; echo READY; read x; exit {code}');loc=barrier(child);foreign=H/('r-'+loc['generation'][:32])/'probe-foreign';foreign.write_bytes(b'UNCHANGED');foreign.chmod(0o600)
 try:
  child.stdin.write(b'go\n');child.stdin.flush();child.wait(timeout=100);out,err=child.communicate();expected=code or 125
  assert child.returncode==expected and not out and b'teardown-failed' in err and foreign.read_bytes()==b'UNCHANGED',(child.returncode,out,err)
  record=json.loads((W/'s/pinfold/runs'/loc['generation']/'teardown.json').read_text());assert record['reason']=='teardown-failed'
  emit(f'cleanup-failure observed={code} client={expected} stderr=teardown-failed run-record=teardown-failed foreign-bytes=unchanged')
 finally:
  if foreign.exists() and foreign.read_bytes()==b'UNCHANGED':foreign.unlink()
  if child.poll() is None:child.terminate();child.communicate(timeout=100)
 run(['stop']);control()
def stop_failure():
 run(['start']);run(['--','sync']);loc=json.loads(L.read_text());foreign=H/('r-'+loc['generation'][:32])/'probe-stop-foreign';foreign.write_bytes(b'UNCHANGED');foreign.chmod(0o600)
 try:
  refused=run(['stop'],125);record=json.loads((W/'s/pinfold/runs'/loc['generation']/'teardown.json').read_text())
  assert b'teardown-failed' in refused.stderr and record['reason']=='teardown-failed' and foreign.read_bytes()==b'UNCHANGED'
  emit('stop-cleanup-failure: stop=125 stderr=teardown-failed run-record=visible-on-return/teardown-failed foreign-bytes=unchanged')
 finally:
  if foreign.exists() and foreign.read_bytes()==b'UNCHANGED':foreign.unlink()
 run(['stop']);control()
try:
 if len(sys.argv)>3 and sys.argv[3]=='stop-only':stop_failure()
 else:
  for code in (0,23):
   if sys.platform=='linux':slow(code)
   else:macslow(code)
  stopped()
  for code in (0,23):cleanup_failure(code)
  stop_failure()
finally:
 try:run(['stop'])
 finally:run(['disk','rm','--all'])
 emit('fixture='+str(W))
