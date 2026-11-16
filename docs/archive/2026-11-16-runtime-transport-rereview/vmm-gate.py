#!/usr/bin/env python3
"""Real musl binary under GDB; every crash is at an external instruction barrier."""
from pathlib import Path
base=Path(__file__).resolve().parent.parent
helpers=(base/'2026-11-16-runtime-transport-review/crash-windows.py').read_text().split('# Anchors intentionally')[0]
exec(compile(helpers,str(base/'2026-11-16-runtime-transport-review/crash-windows.py'),'exec'))
import select
jail=repo/'crates/pinfold/src/core/runtime/firecracker/jail.rs'
module=repo/'crates/pinfold/src/core/runtime/firecracker/mod.rs'

def gated_crash(label,needle,file):
    name=prefix+'-'+label;key=hashlib.sha256(f'box:{name}'.encode()).hexdigest()
    record=host/(key+'.owner');spec=work/(label+'.json');spec.write_text(json.dumps(plan(name))+'\n')
    result=work/(label+'.observed');commands=work/(label+'.gdb')
    commands.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
python
import gdb,json,os,pathlib,select,signal
class Barrier(gdb.Breakpoint):
 def stop(self):
  record=pathlib.Path({str(record)!r})
  if not record.exists():return False
  self.enabled=False
  r=json.loads(record.read_text());state=pathlib.Path(r['state_dir']);children=set()
  for task in pathlib.Path('/proc/'+str(gdb.selected_inferior().pid)+'/task').iterdir():
   children.update(task.joinpath('children').read_text().split())
  observed={{'case':{label!r},'identity':(state/'vmm').read_text(),'temporary':(state/'vmm.tmp').read_text() if (state/'vmm.tmp').exists() else None,'children':[]}}
  handles=[]
  for pid in children:
   p=pathlib.Path('/proc/'+pid)
   observed['children'].append({{'pid':int(pid),'exe':os.readlink(p/'exe'),'wait':(p/'wchan').read_text()}})
   handles.append(os.pidfd_open(int(pid)))
  os.kill(gdb.selected_inferior().pid,signal.SIGKILL)
  for fd in handles:
   assert select.select([fd],[],[],30)[0], 'blocked child survived owner death'
   os.close(fd)
  observed['children_ended']=True
  pathlib.Path({str(result)!r}).write_text(json.dumps(observed))
  return True
Barrier({str(file)+':'+str(anchor(needle,file))!r})
end
run box up < {str(spec)} > {str(work/(label+'.out'))} 2> {str(work/(label+'.err'))}
continue
''')
    completed=sp.run(['gdb','-nx','-q','-batch','-x',str(commands),'--args',binary,'box','up'],env=env,cwd=repo,capture_output=True,timeout=180)
    assert result.exists(),completed.stdout+completed.stderr
    observed=json.loads(result.read_text());emit('VMM-BARRIER '+json.dumps(observed,sort_keys=True))
    assert observed['children_ended']
    if label in ('before-clone','before-identity','temporary-empty','temporary-complete'):
        assert observed['identity']=='not-started\n'
        assert all(Path(c['exe']).name=='pinfold' for c in observed['children'])
    if label!='before-clone':assert len(observed['children'])==1
    if label=='temporary-empty':assert observed['temporary']==''
    if label=='temporary-complete':assert observed['temporary'].endswith('\n')
    if label in ('durable-before-release','released'):
        assert len(observed['identity'].split())==2
    prune();assert not record.exists();corrected_control(name)
    emit(label+': prune and same-name start/exec/down succeeded')

for args in [
 ('before-clone','    let pid = unsafe { nix::sched::clone(',jail),
 ('before-identity','super::record_vmm(state_dir, pid)?;',jail),
 ('temporary-empty','file.write_all(bytes)?;',module),
 ('temporary-complete','file.sync_all()?;',module),
 ('durable-before-release','match nix::unistd::write(&alive_write, &[1])',jail),
 ('released','        let mut filled = 0;',jail),
]:gated_crash(*args)

name=prefix+'-unknown';child=start(name)
record=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
r=json.loads(record.read_text());state=Path(r['state_dir']);identity=state/'vmm';original=identity.read_bytes()
pid=int(original.split()[0]);fd=os.pidfd_open(pid)
child.kill();child.wait(timeout=30);assert select.select([fd],[],[],30)[0];os.close(fd)
for f in [child.stdin,child.stdout,child.stderr]:f.close()
directory=host/('r-'+r['generation'][:32]);marker=(directory/'generation').read_bytes();owner=record.read_bytes()
try:
 for label in ('missing','partial','unreadable'):
  identity.write_bytes(original);os.chmod(identity,0o600)
  if label=='missing':identity.unlink()
  elif label=='partial':identity.write_bytes(original.rstrip(b'\n'))
  else:os.chmod(identity,0)
  refused=run(['box','prune'],code=1)
  assert b'unknown VMM liveness' in refused.stderr
  assert record.read_bytes()==owner and (directory/'generation').read_bytes()==marker
  emit('unknown '+label+': pruning refused, owner record and generation marker unchanged')
  if label=='unreadable':os.chmod(identity,0o600)
finally:
 identity.write_bytes(original);os.chmod(identity,0o600)
prune();corrected_control(name)
emit('PASS durable VMM gate crash windows and unknown-liveness refusal/control')
