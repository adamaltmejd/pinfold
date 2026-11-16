#!/usr/bin/env python3
"""Actual Apple startup helpers: BIN IMAGE [preflight|image|inventory|all]."""
import ctypes
from pathlib import Path
old=Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-rereview/disks.py'
exec(compile(old.read_text().split('def case(')[0].replace('pfrr-disk.','pfr3-start.'),str(old),'exec'),globals())
lib=ctypes.CDLL('/usr/lib/libproc.dylib')
def case(point,action):
 p=W/(point+'-'+action);p.mkdir();ident=p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
 locator=H/(hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()+'.project')
 launch=H/(hashlib.sha256(('project-launch:'+hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()).encode()).hexdigest()+'.lock')
 run(p,['config','edit']);c=W/'c/pinfold/projects'/ident/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
 starter=sp.Popen([B,'--','printf','MUST-NOT-RUN'],cwd=p,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
 queue=select.kqueue();note(queue,starter.pid);tracked=set();paused=set();owner=native=None
 def resume(pid):
  try:os.kill(pid,signal.SIGCONT)
  except ProcessLookupError:pass
  paused.discard(pid)
 def candidates(parent,path):
  ids=(ctypes.c_int*256)();lib.proc_listchildpids(parent,ids,ctypes.sizeof(ids))
  pending={pid for pid in ids if pid and pid not in tracked}
  for pid in pending:
   try:note(queue,pid);tracked.add(pid)
   except ProcessLookupError:pass
  pending.update(event.ident for event in queue.control([],64,0) if event.ident!=starter.pid and event.fflags&select.KQ_NOTE_EXEC)
  result=[]
  for pid in pending:
   try:os.kill(pid,signal.SIGSTOP);paused.add(pid)
   except ProcessLookupError:continue
   binary=ctypes.create_string_buffer(4096);lib.proc_pidpath(pid,binary,len(binary))
   if not binary.value.endswith(path):resume(pid);continue
   fields=sp.run(['ps','-p',str(pid),'-o','ppid=,command='],capture_output=True,timeout=5).stdout.decode().strip().split(None,1)
   if len(fields)!=2 or int(fields[0])!=parent:resume(pid);continue
   result.append((pid,fields[1]))
  return result
 try:
  deadline=time.monotonic()+120
  while owner is None:
   assert time.monotonic()<deadline and starter.poll() is None,'owner barrier absent'
   for pid,args in candidates(starter.pid,os.fsencode(B)):
    if '__project-owner' in args:owner=pid;note(queue,pid)
    resume(pid)
  while native is None:
   assert time.monotonic()<deadline and starter.poll() is None,'native startup barrier absent'
   for pid,args in candidates(owner,b'/cli/container'):
    if not locator.exists():resume(pid);continue
    loc=json.loads(locator.read_text());claim=H/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner')
    match={'preflight':'container --version','image':'container image inspect ','inventory':'container list --all --format json'}[point]
    if match not in args or not claim.exists():resume(pid);continue
    native=pid;note(queue,pid);stopping=time.monotonic()+2
    while b'T' not in sp.run(['ps','-p',str(pid),'-o','state='],capture_output=True,check=True).stdout:
     assert time.monotonic()<stopping,'native helper not stopped'
    observed=json.loads(claim.read_text());assert observed['disks']==[] and observed['described'] is None
    break
  began=time.monotonic();assert not select.select([starter.stdout,starter.stderr],[],[],12)[0] and starter.poll() is None,'bootstrap stopped progressing before cancellation'
  assert time.monotonic()-began>=12
  began=time.monotonic()
  if action=='stop':run(p,['stop'])
  else:starter.send_signal(signal.SIGTERM)
  out,err=starter.communicate(timeout=60);expected=125 if action=='stop' else 143
  assert starter.returncode==expected and not out and b'owner-timeout' not in err,(starter.returncode,out,err)
  if action=='stop':assert b'stopped' in err,err
  wait_dead(queue,{owner,native},30);paused.discard(native);elapsed=time.monotonic()-began
  assert not locator.exists() and not claim.exists()
  import fcntl
  with launch.open('rb') as lock:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
  assert run(p,['--','sh','-c','printf STARTUP-CONTROL; sync']).stdout==b'STARTUP-CONTROL'
  emit(f'{point}-{action}: real native helper SIGSTOP; Starting sustained bootstrap beyond12s; stop='+('0' if action=='stop' else 'not invoked')+f' client={expected}; CMD=absent; owner/helper kernel exit; claim/locator=absent launch=available; teardown={elapsed:.2f}s; next-start=0/STARTUP-CONTROL')
 finally:
  for pid in list(paused):resume(pid)
  if starter.poll() is None:starter.terminate();starter.communicate(timeout=60)
  queue.close();run(p,['stop'])
  for disk in ('pi-'+ident+'-home','pi-'+ident+'-project'):run(p,['disk','rm',disk])
for point in ('preflight','image','inventory'):
 if MODE in ('all',point):
  for action in ('stop','signal'):case(point,action)
emit('cleanup exact fixture boxes/disks=True')
