#!/usr/bin/env python3
"""Linux real owner/exec race: BIN IMAGE baseline|fixed [fresh-root]."""
import ctypes, hashlib, json, os, select, signal, subprocess as sp, sys, tempfile, time
from pathlib import Path
assert sys.platform.startswith('linux'), 'Linux proc/ptrace observer only'
binary=str(Path(sys.argv[1]).resolve()); image,phase=sys.argv[2:4]
work=Path(sys.argv[4] if len(sys.argv)>4 else tempfile.mkdtemp(prefix='pfready.',dir='/tmp')).resolve()
env={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(work/'c'),XDG_STATE_HOME=str(work/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G')
(work/'s/pinfold').mkdir(parents=True);(work/'s/pinfold/maintenance').touch()
root=work/'probe-ready';root.mkdir()
sp.run(['git','init','-q',str(root)],check=True,capture_output=True)
ident=root.name+'-'+hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg=work/'c/pinfold/projects'/ident/'pinfold.toml';cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = '+json.dumps(image)+'\n')
libc=ctypes.CDLL(None,use_errno=True);libc.ptrace.restype=ctypes.c_long
libc.ptrace.argtypes=[ctypes.c_uint,ctypes.c_uint,ctypes.c_void_p,ctypes.c_void_p]
def ptrace(request,pid):
    if libc.ptrace(request,pid,None,None)<0: raise OSError(ctypes.get_errno(),os.strerror(ctypes.get_errno()))
def emit(value): print(value.replace(binary,'$BIN').replace(str(work),'$W'),flush=True)
def wait_for(probe,description):
    deadline=time.monotonic()+45
    while time.monotonic()<deadline:
        value=probe()
        if value: return value
        os.sched_yield()
    raise RuntimeError('readiness deadline: '+description)
def tasks(pid):
    result={}
    for task in Path(f'/proc/{pid}/task').glob('*'):
        try: result[int(task.name)]=(task.joinpath('comm').read_text().strip(),task.joinpath('wchan').read_text().strip())
        except FileNotFoundError: pass
    return result
def exec_worker(pid):
    observed=tasks(pid)
    found=[tid for tid,(name,where) in observed.items() if name.startswith('tokio-') and any(token in where for token in ['unix', 'skb_wait'])]
    return found[0] if len(found)==1 and any(name=='exec-input' for name,_ in observed.values()) else None
def worker_parked(pid,tid):
    observed=tasks(pid)
    state=observed.get(tid)
    return state and 'futex' in state[1] and not any(name=='exec-input' for name,_ in observed.values())
def stopped_main(pid):
    observed=os.waitpid(pid,os.WNOHANG|os.WUNTRACED|0x40000000)
    return observed if observed[0] else None
def down_waiting(child):
    if child.poll() is not None: raise RuntimeError('box down returned before owner resumed')
    try: return any(token in Path(f'/proc/{child.pid}/wchan').read_text() for token in ['unix', 'skb_wait'])
    except FileNotFoundError: return False
try:
    for expected in [23,127]:
        child=down=None;attached=False
        try:
            child=sp.Popen([binary,'--','sh','-c','echo CMD-READY; read gate; exit "$1"','sh',str(expected)],cwd=root,env=env,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
            assert select.select([child.stdout],[],[],60)[0], 'command readiness deadline'
            assert child.stdout.readline()==b'CMD-READY\n'
            worker=wait_for(lambda:exec_worker(child.pid),'exec worker waiting on transport')
            try: ptrace(0x4206,child.pid);attached=True;ptrace(0x4207,child.pid)
            except OSError as error:
                emit(f'PTRACE-UNAVAILABLE errno={error.errno} detail={error.strerror}');raise
            stopped=wait_for(lambda:stopped_main(child.pid),'main-TID trace stop')
            assert stopped[0]==child.pid and os.WIFSTOPPED(stopped[1])
            child.stdin.write(b'finish\n');child.stdin.flush()
            wait_for(lambda:worker_parked(child.pid,worker),'exec-input gone and selected worker parked')
            name=f'pi-{ident}-{child.pid}'
            down=sp.Popen([binary,'box','down',name],cwd=root,env=env,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
            wait_for(lambda:down_waiting(down),'public down waiting for reply')
            emit(f'phase={phase} expected={expected} main-TID-stopped=true worker={worker} socket-read-to-futex=true exec-input-gone=true down-request-waiting=true')
            ptrace(17,child.pid);attached=False
            stdout,stderr=child.communicate(timeout=60);downout,downerr=down.communicate(timeout=60)
            emit(f'$ $BIN -- sh -c \'echo CMD-READY; read gate; exit "$1"\' sh {expected}; public box down pending before main resumes')
            emit(f'project-exit={child.returncode} stdout={stdout!r} stderr={stderr!r}; down-exit={down.returncode} stdout={downout!r} stderr={downerr!r}')
            if phase=='fixed': assert child.returncode==expected
            else: assert child.returncode in {expected,125}
            assert down.returncode==0
        finally:
            if attached:
                try: ptrace(17,child.pid)
                except OSError: pass
            if child is not None and child.poll() is None:
                child.terminate();child.communicate(timeout=60)
            if down is not None and down.poll() is None: down.communicate(timeout=60)
finally:
    cleaned=sp.run([binary,'disk','rm','--all','--json'],cwd=root,env=env,capture_output=True,timeout=60)
    assert cleaned.returncode==0
    emit('isolated disks removed')
emit('PASS completed-exec/down scheduling case')
