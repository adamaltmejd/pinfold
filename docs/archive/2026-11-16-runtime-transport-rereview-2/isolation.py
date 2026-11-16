#!/usr/bin/env python3
"""Linux two-box readiness and inventory races, with real controls."""
from pathlib import Path
source=Path(__file__).with_name('retirement.py')
exec(compile(source.read_text().split('ownership = repo')[0],str(source),'exec'))
import select
module=repo/'crates/pinfold/src/core/runtime/firecracker/mod.rs'

def start(name,disk=False):
    child=sp.Popen([binary,'box','up'],env=env,cwd=work,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
    spec={'name':name,'image':image,'cpus':2,'memory':'1G','egress':{'allow':[]}}
    if disk:spec['disks']=[{'name':prefix+'-data','guest':'/data','size':'1G'}]
    child.stdin.write(json.dumps(spec).encode()+b'\n');child.stdin.flush()
    ready=json.loads(child.stdout.readline());assert ready['event']=='ready',ready
    owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
    return child,owner,json.loads(owner.read_text())

def kill_owner(child,record):
    identity=Path(record['state_dir'])/'vmm';pid=int(identity.read_text().split()[0]);fd=os.pidfd_open(pid)
    child.kill();child.wait(timeout=30);assert select.select([fd],[],[],30)[0];os.close(fd)
    for f in (child.stdin,child.stdout,child.stderr):f.close()

def observed_start(name,other_identity):
    # Keep stdin and stdout open through real FIFO endpoints. Read readiness
    # from the actual CLI, then exec and stop from another process.
    input_path=work/'input.fifo';output_path=work/'output.fifo'
    os.mkfifo(input_path);os.mkfifo(output_path)
    incoming=os.open(input_path,os.O_RDWR);outgoing=os.open(output_path,os.O_RDWR|os.O_NONBLOCK)
    spec={'name':name,'image':image,'cpus':2,'memory':'1G','egress':{'allow':[]}}
    os.write(incoming,json.dumps(spec).encode()+b'\n')
    location=[i for i,l in enumerate(module.read_text().splitlines(),1) if 'let record = Self::record(name)?;' in l][0]
    commands=work/'ready.gdb';observed=work/'ready.observed'
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
python
import gdb,pathlib
class Ready(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  pathlib.Path({str(other_identity)!r}).chmod(0)
  pathlib.Path({str(observed)!r}).write_text('other identity made unreadable at own finish_start after guest READY')
  return False
Ready({str(module)+':'+str(location)!r})
end
run box up < {input_path} > {output_path} 2> {work/'ready.err'}
''')
    debugger=sp.Popen(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary,'box','up'],env=env,cwd=repo,stdout=sp.PIPE,stderr=sp.PIPE)
    try:
        assert select.select([outgoing],[],[],120)[0],'healthy start gave no readiness'
        ready=json.loads(os.read(outgoing,65536).splitlines()[0]);assert ready['event']=='ready',ready
        assert observed.exists()
        assert run(['box','exec',name,'--','printf','UNRELATED-READY-CONTROL']).stdout==b'UNRELATED-READY-CONTROL'
        run(['box','down',name]);out,err=debugger.communicate(timeout=90)
        assert debugger.returncode==0,(out,err)
        emit('2 other identity unreadable at healthy finish_start after guest READY: healthy ready/exec/down exit0')
    finally:
        if debugger.poll() is None:debugger.kill();debugger.wait()
        os.close(incoming);os.close(outgoing)

name=prefix+'-unknown';child,owner,record=start(name,disk=True)
identity=Path(record['state_dir'])/'vmm';kill_owner(child,record)
try:
    observed_start(prefix+'-healthy',identity)
    listed=run(['box','list','--label','dev.pinfold.owner'])
    row=next(json.loads(l) for l in listed.stdout.splitlines() if json.loads(l)['name']==name)
    assert row['state']=='unknown' and b'unknown VMM liveness' in listed.stderr
    refused=sp.run([binary,'disk','rm',prefix+'-data','--json'],env=env,capture_output=True,timeout=60)
    assert refused.returncode==1 and b'disk-in-use' in refused.stderr+refused.stdout
    emit('2 unknown identity inventory reports unknown, preserves owner/marker and refuses its disk removal as disk-in-use')
finally:identity.chmod(0o600)
run(['box','down',name]);run(['disk','rm',prefix+'-data','--json'])

# A real inventory snapshot is held while a different process retires exactly
# that generation and removes its identity tree and claim.
name=prefix+'-snapshot';child,owner,record=start(name);kill_owner(child,record)
witness=prefix+'-witness';live,_,_=start(witness)
location=[i for i,l in enumerate(module.read_text().splitlines(),1) if 'match vmm_state(&record.state_dir)' in l];assert len(location)==1
commands=work/'inventory.gdb';observation=work/'inventory.observed'
commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
python
import gdb,pathlib,subprocess
class Snapshot(gdb.Breakpoint):
 def stop(self):
  self.enabled=False
  result=subprocess.run([{binary!r},'box','down',{name!r}],capture_output=True,timeout=60)
  assert result.returncode==0,result.stderr
  assert not pathlib.Path({str(owner)!r}).exists()
  assert not pathlib.Path({record['state_dir']!r}).exists()
  pathlib.Path({str(observation)!r}).write_text('claim and identity tree retired after inventory snapshot')
  return False
Snapshot({str(module)+':'+str(location[0])!r})
end
run box list --label dev.pinfold.owner > {work/'inventory.out'} 2> {work/'inventory.err'}
''')
try:
    r=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary,'box','list'],env=env,cwd=repo,capture_output=True,timeout=120)
    assert r.returncode==0 and observation.exists(),r.stdout+r.stderr
    rows=[json.loads(l) for l in (work/'inventory.out').read_bytes().splitlines()]
    assert name not in [row['name'] for row in rows] and witness in [row['name'] for row in rows]
    assert b'unknown VMM liveness' not in (work/'inventory.err').read_bytes()
    assert run(['box','exec',witness,'--','printf','LIVE-WITNESS']).stdout==b'LIVE-WITNESS'
    emit('2 inventory snapshot -> concurrent down removed state/claim -> recheck skipped retired generation; live witness exec0')
finally:
    run(['box','down',witness]);live.communicate(timeout=90)
control(name)
emit('PASS isolated readiness, unknown identity/disk protection and retirement snapshot race controls')
