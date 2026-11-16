#!/usr/bin/env python3
"""Observe real post-command maintenance: maintenance.py BIN IMAGE (Linux)."""
import pathlib
old=pathlib.Path(__file__).resolve().with_name('admission.py')
exec(compile(old.read_text().split('def maintenance_case():')[0],str(old),'exec'),globals())
sp.run(['git','init','-q',str(P)],env=E,check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],env=E,check=True)
run(['config','edit']);config=W/'c/pinfold/projects'/ID/'pinfold.toml';config.write_text(config.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
 out=W/'command.out'
 d=debugger('post-command-maintenance',['--','printf','CMD-CONTROL'],'pinfold::core::clean::maintain',stdout=out)
 assert out.read_bytes()==b'CMD-CONTROL','maintenance preceded observed command output'
 assert not L.exists(),'maintenance retained a project lease/locator'
 launches=list((W/'s/pinfold/runs').glob('*/launch.json'));assert len(launches)==1,'fixture did not have exactly one completed run'
 launch=json.loads(launches[0].read_text());name=launch['box'];key=hashlib.sha256(('box:'+name).encode()).hexdigest()
 assert not (H/(key+'.owner')).exists(),'maintenance retained the named claim'
 with (H/(key+'.lock')).open('rb') as lock:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
 terminal=json.loads(launches[0].with_name('terminal.json').read_text());assert terminal['reason']=='sessions-ended' and terminal['detail'] is None
 assert run(['--','printf','NEXT-CONTROL']).stdout==b'NEXT-CONTROL','paused maintenance prevented the next project command'
 assert d['process'].poll() is None,'first client left its maintenance barrier'
 emit('maintenance-entry: first stdout=CMD-CONTROL; locator=absent claim=absent name-flock=available terminal=sessions-ended/null; paused client held no lease/launch; second command=0/NEXT-CONTROL')
 release(d);finish(d,0)
 emit('maintenance-control: first client resumed=0; fresh maintenance stamp preserved; no global cleanup/pruning exercised')
finally:
 for d in debugs:release(d)
 try:run(['stop'])
 finally:
  for d in debugs:
   if d['process'].poll() is None:d['process'].terminate();d['process'].communicate(timeout=30)
   os.close(d['ready']);os.close(d['go'])
  run(['disk','rm','--all'])
 emit('fixture=$W; scoped cleanup complete')
