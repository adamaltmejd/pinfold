#!/usr/bin/env python3
"""Real no-replace publication failure cleans only private pre-start staging."""
from pathlib import Path
helper=Path(__file__).resolve().parent.parent/'2026-11-16-runtime-transport-rereview-2/retirement.py'
exec(compile(helper.read_text().split('ownership = repo')[0],str(helper),'exec'))
name=prefix+'-staging';owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
source=repo/'crates/pinfold/src/core/runtime/sockets.rs'
line=[i for i,l in enumerate(source.read_text().splitlines(),1) if 'let renamed = rename_at(' in l];assert len(line)==1
spec=work/'spec.json';spec.write_text(json.dumps({'name':name,'image':image,'cpus':2,'memory':'1G'})+'\n')
observation=work/'observed.json';commands=work/'staging.gdb'
commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
python
import gdb,json,pathlib
owner=pathlib.Path({str(owner)!r});base=owner.parent
class Returned(gdb.FinishBreakpoint):
 def stop(self):
  record=json.loads(owner.read_text());generation=record['generation']
  stage=base/('.r-'+generation);foreign=base/('r-'+generation[:32])
  assert not record['retiring'] and not stage.exists()
  assert (pathlib.Path(record['state_dir'])/'vmm').read_text()=='not-started\\n'
  assert (foreign/'sentinel').read_bytes()==b'FOREIGN-COLLISION'
  result={{'generation':generation,'retiring':False,'private_stage_absent':True,'foreign_bytes_unchanged':True,'vmm':'not-started'}}
  pathlib.Path({str(observation)!r}).write_text(json.dumps(result))
  gdb.write('PRESTART-STAGING '+json.dumps(result)+'\\n')
  return False
class Collision(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  record=json.loads(owner.read_text());assert not record['retiring']
  foreign=base/('r-'+record['generation'][:32]);foreign.mkdir(mode=0o700)
  (foreign/'sentinel').write_bytes(b'FOREIGN-COLLISION')
  Returned(gdb.newest_frame(),internal=True)
  return False
Collision({str(source)+':'+str(line[0])!r})
end
run box up < {spec} > {work/'out'} 2> {work/'err'}
''')
result=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary,'box','up'],env=env,cwd=repo,capture_output=True,timeout=120)
assert observation.exists(),result.stdout+result.stderr
observed=json.loads(observation.read_text());foreign=host/('r-'+observed['generation'][:32])
try:
 emit('PRESTART-STAGING '+json.dumps(observed))
 event=json.loads((work/'out').read_bytes());assert event['event']=='refused' and event['reason']=='runtime-path',event
 assert not owner.exists() and (foreign/'sentinel').read_bytes()==b'FOREIGN-COLLISION'
 emit('real publication refused runtime-path; private staging removed while claim was not retiring; foreign bytes preserved')
finally:
 (foreign/'sentinel').unlink();foreign.rmdir()
control(name)
emit('PASS pre-start private staging exception and corrected same-name exec/down control')
