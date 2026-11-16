#!/usr/bin/env python3
"""Real Apple failed-deletion recovery with a corrected filesystem control."""
from pathlib import Path
source=Path(__file__).with_name('retirement.py')
exec(compile(source.read_text().split('ownership = repo')[0],str(source),'exec'))
name=prefix+'-apple'
child=sp.Popen([binary,'box','up'],env=env,cwd=work,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
child.stdin.write(json.dumps({'name':name,'image':image,'cpus':2,'memory':'1G','egress':{'allow':[]}}).encode()+b'\n');child.stdin.flush()
ready=json.loads(child.stdout.readline());assert ready['event']=='ready',ready
owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
record=json.loads(owner.read_text());state=Path(record['state_dir'])
(state/'fixture').write_bytes(b'RETIREMENT-FIXTURE');os.chmod(state,0o500)
try:
 child.stdin.close();child.stdin=None
 out,err=child.communicate(timeout=120)
 emit(f'owned EOF with state deletion refused: exit={child.returncode} stdout={out!r} stderr={err!r}')
 assert child.returncode==1 and 'Permission denied' in json.loads(out)['detail']
 retired=json.loads(owner.read_text())
 assert retired['generation']==ready['run_id'] and retired['retiring'] is True
 assert (state/'fixture').read_bytes()==b'RETIREMENT-FIXTURE'
 listed=sp.check_output(['container','list','--all','--format','json'])
 assert name not in [r.get('configuration',{}).get('id',r.get('id')) for r in json.loads(listed)]
 emit('real Apple runtime absent; durable matching owner retiring=true survives failed state deletion')
finally:
 os.chmod(state,0o700)
run(['box','down',name]);assert not owner.exists() and not state.exists()
control(name)
emit('PASS Apple owned retirement, failed-delete preservation, repaired down and fresh same-name exec')
