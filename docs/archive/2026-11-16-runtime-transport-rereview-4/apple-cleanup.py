#!/usr/bin/env python3
"""Confirmed Apple helper removal restores failed-start cleanup."""
from pathlib import Path
source=Path(__file__).resolve().parent.parent/'2026-11-16-runtime-transport-rereview-2/retirement.py'
exec(compile(source.read_text().split('ownership = repo')[0],str(source),'exec'))
name=prefix+'-prepare';disk=name+'-data'
seed=work/'seed';seed.mkdir();(seed/'trap').symlink_to('/tmp/preparation-must-not-traverse')
share=work/'share';share.mkdir();(share/'fixture').write_bytes(b'PREPARATION-CONTROL')
env['PROBE_KEY']='fixture-only-unused'
spec={'name':name,'image':image,'cpus':2,'memory':'1G','env':{'HOME':'/data/trap/home'},'egress':{'routes':{'probe':{'login':'pi','provider':'openai','from':'PROBE_KEY','to':'http://127.0.0.1:9'}}},'disks':[{'name':disk,'guest':'/data','size':'1G','from':str(seed)}]}
owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
state=work/'s/pinfold/boxes'/hashlib.sha256(name.encode()).hexdigest()[:16]
def up():
 p=sp.Popen([binary,'box','up'],env=env,cwd=work,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
 p.stdin.write(json.dumps(spec).encode()+b'\n');p.stdin.flush()
 return p,json.loads(p.stdout.readline())
try:
 p,event=up();out,err=p.communicate(timeout=120)
 emit(f'real preparation failure: exit={p.returncode} event={event!r} stderr={err!r}')
 assert p.returncode==1 and event['event']=='failed'
 assert not owner.exists() and not state.exists()
 listed=run(['disk','list','--json']);assert disk not in [json.loads(line)['name'] for line in listed.stdout.splitlines()]
 emit('removed failing preparation helper left no claim, state or new disk pinned')
 (seed/'trap').unlink();(seed/'trap').mkdir()
 spec['labels']={'probe-error':'bad=value'}
 p,event=up();out,err=p.communicate(timeout=120)
 assert p.returncode==1 and event['event']=='failed' and not owner.exists() and not state.exists()
 listed=run(['disk','list','--json']);assert disk not in [json.loads(line)['name'] for line in listed.stdout.splitlines()]
 emit('native run refused before container creation; new claim and disk cleaned')
 spec.pop('labels')
 p,event=up();assert event['event']=='ready',event
 assert run(['box','exec',name,'--','printf','PREPARATION-CONTROL']).stdout==b'PREPARATION-CONTROL'
 run(['box','down',name]);p.communicate(timeout=120);assert p.returncode==0
 run(['disk','rm',disk,'--json'])
 emit('PASS Apple failed preparation and no-container startup cleanup, corrected same-name helper/exec/down')
finally:
 run(['box','down',name]);run(['disk','rm',disk,'--json'])
