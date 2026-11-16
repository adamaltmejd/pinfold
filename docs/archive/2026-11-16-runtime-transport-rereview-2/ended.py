#!/usr/bin/env python3
"""Owned already-ended VMM status remains a required retirement proof."""
from pathlib import Path
source=Path(__file__).with_name('retirement.py')
exec(compile(source.read_text().split('ownership = repo')[0],str(source),'exec'))
import select,signal
name=prefix+'-ended'
child=sp.Popen([binary,'box','up'],env=env,cwd=work,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
child.stdin.write(json.dumps({'name':name,'image':image,'cpus':2,'memory':'1G','egress':{'allow':[]}}).encode()+b'\n');child.stdin.flush()
ready=json.loads(child.stdout.readline());assert ready['event']=='ready',ready
owner=host/(hashlib.sha256(f'box:{name}'.encode()).hexdigest()+'.owner')
record=json.loads(owner.read_text());state=Path(record['state_dir'])
pid=int((state/'vmm').read_text().split()[0]);fd=os.pidfd_open(pid)
os.kill(pid,signal.SIGKILL);assert select.select([fd],[],[],30)[0];os.close(fd)
out=child.stdout.readline();child.wait(timeout=90);err=child.stderr.read()
child.stdin.close();child.stdout.close();child.stderr.close()
emit(f'owned VMM killed at ready: exit={child.returncode} stdout={out!r} stderr={err!r}')
assert child.returncode==1 and json.loads(out)['reason']=='exited'
assert not owner.exists() and not state.exists()
control(name)
emit('PASS already-ended owned VMM awaited, retirement completed, same-name execution control succeeded')
