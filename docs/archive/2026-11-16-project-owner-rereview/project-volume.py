#!/usr/bin/env python3
"""Mac project stop/signal during real volume work: project-volume.py BIN IMAGE [stop|signal|all]."""
import ctypes
from pathlib import Path
base=Path(__file__).with_name('disks.py')
exec(compile(base.read_text().split('def case(')[0],str(base),'exec'),globals())
lib=ctypes.CDLL('/usr/lib/libproc.dylib')

def case(action):
 p=W/('project-volume-'+action);p.mkdir();ident=p.name+'-'+hashlib.sha256(os.fsencode(p)).hexdigest()[:12]
 locator=H/(hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()+'.project')
 launch=H/(hashlib.sha256(('project-launch:'+hashlib.sha256(b'project-owner:'+os.fsencode(p)).hexdigest()).encode()).hexdigest()+'.lock')
 run(p,['config','edit']);config=W/'c/pinfold/projects'/ident/'pinfold.toml';config.write_text(config.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
 starter=sp.Popen([B,'--','sh','-c','printf MUST-NOT-RUN'],cwd=p,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
 queue=select.kqueue();note(queue,starter.pid);tracked=set();paused=set();owner=native=None;seen=set()
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
   assert time.monotonic()<deadline and starter.poll() is None,'required detached owner not observed'
   for pid,args in candidates(starter.pid,os.fsencode(B)):
    if '__project-owner' in args:owner=pid;note(queue,pid)
    resume(pid)
  deadline=time.monotonic()+120
  while native is None:
   assert time.monotonic()<deadline and starter.poll() is None,'required actual project volume command not observed'
   for pid,args in candidates(owner,b'/cli/container'):
    seen.add(args)
    if 'container volume ' not in args:resume(pid);continue
    native=pid;note(queue,pid);stopping=time.monotonic()+2
    while True:
     state=sp.run(['ps','-o','state=','-p',str(pid)],capture_output=True,check=True,timeout=5).stdout
     if b'T' in state:break
     assert time.monotonic()<stopping,'required native helper did not stop'
    assert locator.exists(),'project locator absent during preparation'
    observed=json.loads(locator.read_text());claim=H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner')
    assert claim.exists(),'project disk-preparation claim absent'
    break
  interrupted=time.monotonic()
  if action=='stop':run(p,['stop'])
  else:os.kill(starter.pid,signal.SIGTERM)
  out,err=starter.communicate(timeout=650);expected=125 if action=='stop' else 143
  assert starter.returncode==expected and not out,(starter.returncode,out,err[:300])
  if action=='stop':assert b'stopped' in err and b'owner-timeout' not in err,err
  wait_dead(queue,{owner,native},max(1,610-(time.monotonic()-interrupted)));paused.discard(native)
  elapsed=time.monotonic()-interrupted;assert elapsed<610,'preparation exceeded bounded cleanup'
  assert not locator.exists() and not claim.exists(),'owner left its claim or locator'
  import fcntl
  with launch.open('rb') as lock:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
  control=run(p,['--','sh','-c','printf PROJECT-VOLUME-CONTROL; sync']);assert control.stdout==b'PROJECT-VOLUME-CONTROL'
  assert not locator.exists(),'control departure retained locator'
  volumes=pathlib.Path(HOME)/'Library/Application Support/com.apple.container/volumes'
  owned=[entry for entry in volumes.iterdir() if any(entry.name.endswith('-'+disk) for disk in ('pi-'+ident+'-home','pi-'+ident+'-project'))]
  emit('project-volume allocation sample: owned entries='+str(len(owned))+' KiB='+str(sum(int(sp.run(['du','-sk',str(entry)],capture_output=True,check=True).stdout.split()[0]) for entry in owned)))

  emit(f'project-volume-{action}: actual native volume command SIGSTOP confirmed; stop='+('0' if action=='stop' else 'not invoked')+f'; launcher={expected}; CMD=absent; owner/helper kernel NOTE_EXIT; claim/locator=absent launch=available; seconds={elapsed:.2f}; same-fixture restart=0/PROJECT-VOLUME-CONTROL')
 finally:
  for pid in list(paused):resume(pid)
  if starter.poll() is None:starter.terminate();starter.communicate(timeout=650)
  queue.close();run(p,['stop'])
  for disk in ('pi-'+ident+'-home','pi-'+ident+'-project'):run(p,['disk','rm',disk])
  emit(f'project-volume-{action}: own boxes/disks removed; fixture='+str(p))

for action in ('stop','signal'):
 if MODE in ('all',action):case(action)
