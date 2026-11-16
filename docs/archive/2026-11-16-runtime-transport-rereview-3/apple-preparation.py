#!/usr/bin/env python3
"""A real Apple preparation-helper failure keeps disks pinned until retirement."""
from pathlib import Path
source=Path(__file__).resolve().parent.parent/'2026-11-16-runtime-transport-rereview-2/retirement.py'
exec(compile(source.read_text().split('ownership = repo')[0],str(source),'exec'))
name=prefix+'-prepare';disk=name+'-data'
seed=work/'seed';seed.mkdir();(seed/'trap').symlink_to('/tmp/preparation-must-not-traverse')
share=work/'share';share.mkdir();(share/'fixture').write_bytes(b'PREPARATION-CONTROL')
env['PROBE_KEY']='fixture-only-unused'
spec={'name':name,'image':image,'cpus':2,'memory':'1G','env':{'HOME':'/data/trap/home'},'egress':{'routes':{'probe':{'login':'pi','provider':'openai','from':'PROBE_KEY','to':'http://127.0.0.1:9'}}},'disks':[{'name':disk,'guest':'/data','size':'1G','from':str(seed)}]}
owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
def up():
 p=sp.Popen([binary,'box','up'],env=env,cwd=work,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
 p.stdin.write(json.dumps(spec).encode()+b'\n');p.stdin.flush()
 return p,json.loads(p.stdout.readline())
try:
 p,event=up();out,err=p.communicate(timeout=120)
 emit(f'real preparation failure: exit={p.returncode} event={event!r} stderr={err!r}')
 assert p.returncode==1 and event['event']=='failed'
 r=json.loads(owner.read_text());assert not r['retiring'] and Path(r['state_dir']).exists()
 refused=sp.run([binary,'disk','rm',disk,'--json'],env=env,capture_output=True,timeout=60)
 assert refused.returncode==1 and b'disk-in-use' in refused.stdout+refused.stderr
 emit('failed helper left nonretiring claim/state and disk-in-use; no unsafe rollback')
 run(['box','down',name]);assert not owner.exists()
 (seed/'trap').unlink();(seed/'trap').mkdir()
 p,event=up();assert event['event']=='ready',event
 assert run(['box','exec',name,'--','printf','PREPARATION-CONTROL']).stdout==b'PREPARATION-CONTROL'
 run(['box','down',name]);p.communicate(timeout=120);assert p.returncode==0
 run(['disk','rm',disk,'--json'])
 emit('PASS Apple preparation failure preserved resources, targeted retirement and corrected same-name helper/exec/down succeeded')
finally:
 run(['box','down',name]);run(['disk','rm',disk,'--json'])
