#!/usr/bin/env python3
"""Real Apple CLI review cases: BIN IMAGE [main809-binary]."""
import ctypes, fcntl, hashlib, json, os, re, select, signal, subprocess as sp, sys, tempfile, termios, time
from pathlib import Path
binary=str(Path(sys.argv[1]).resolve()); image=sys.argv[2]
work=Path(tempfile.mkdtemp(prefix='pfrvapple.',dir='/tmp')).resolve()
root=work/'probe-review'; root.mkdir()
for args in [['init','-q'],['-c','user.name=user','-c','user.email=user@example.invalid','commit','--allow-empty','-qm','fixture']]:
    sp.run(['git','-C',str(root)]+args,check=True,capture_output=True)
ident=root.name+'-'+hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
env={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(work/'c'),XDG_STATE_HOME=str(work/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G')
(work/'s/pinfold').mkdir(parents=True);(work/'s/pinfold/maintenance').touch()
cfg=work/'c/pinfold/projects'/ident/'pinfold.toml';cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{host=".",kind="disk"}]\nimage='+json.dumps(image)+'\n')
def emit(s):print(s.replace(str(work),'$W').replace(binary,'$BIN').replace(str(Path.home()),'/Users/user'),flush=True)
def run(args,data=b'',code=0):
    r=sp.run([binary]+args,cwd=root,env=env,input=data,capture_output=True,timeout=90)
    assert r.returncode==code,(args,r.returncode,r.stderr[:200])
    return r

def pipe_case(bin,stream=1):
    command='exec yes PIPE'+(' >&2' if stream==2 else '')
    r=sp.Popen([bin,'--','sh','-c',command],cwd=root,env=env,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
    source=r.stdout if stream==1 else r.stderr
    assert source.read(1)
    source.close()
    code=r.wait(timeout=30)
    for f in [r.stdout,r.stderr]:
        if not f.closed:f.close()
    return code

if len(sys.argv)>3:
    before=[pipe_case(str(Path(sys.argv[3]).resolve()),i) for i in (1,2)]
    after=[pipe_case(binary,i) for i in (1,2)]
    emit(f'F4 closed stdout/stderr: main809={before}; fixed={after}; no infrastructure125')
    assert after==before and 125 not in after

# A caller can supply a full nonblocking stderr pipe without any binary hook.
read,write=os.pipe();os.set_blocking(write,False)
try:
    while True:os.write(write,b'F'*4096)
except BlockingIOError:pass
try:
    failure=sp.Popen([binary,'--'],cwd=root,env=env,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=write)
    assert failure.wait(timeout=15)==125
    control=sp.Popen([binary,'--','printf','NB-CONTROL'],cwd=root,env=env,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=write)
    out=control.communicate(timeout=60)[0]
    assert control.returncode==0 and out==b'NB-CONTROL'
    emit('F6 full nonblocking stderr: diagnostic failure exits125 without panic; same-descriptor command control exits0')
finally:
    os.close(read);os.close(write)

lib=ctypes.CDLL('/usr/lib/libproc.dylib')
def exec_pid(parent):
    pids=(ctypes.c_int*64)();count=lib.proc_listchildpids(parent,pids,ctypes.sizeof(pids))
    for pid in pids[:count]:
        if not pid:continue
        args=sp.check_output(['ps','-p',str(pid),'-o','args=']).decode()
        if ' exec -i' in args:return pid
    raise RuntimeError('native exec child not observed at guest FIFO barrier')

def completed(code,send_signal=True):
    read,write=os.pipe();os.set_blocking(write,False)
    try:
        while True:os.write(write,b'F'*4096)
    except BlockingIOError:pass
    os.set_blocking(write,True)
    script=f'mkfifo /tmp/review-gate; echo REVIEW-READY >&2; cat /tmp/review-gate >/dev/null; printf DONE; exit {code}'
    child=sp.Popen([binary,'--','sh','-c',script],cwd=root,env=env,stdin=sp.DEVNULL,stdout=write,stderr=sp.PIPE)
    os.close(write);pending=b''
    try:
        while b'REVIEW-READY\n' not in pending:
            assert select.select([child.stderr],[],[],60)[0]
            pending+=os.read(child.stderr.fileno(),4096)
        native=exec_pid(child.pid);queue=select.kqueue()
        queue.control([select.kevent(native,filter=select.KQ_FILTER_PROC,flags=select.KQ_EV_ADD|select.KQ_EV_ONESHOT,fflags=select.KQ_NOTE_EXIT)],0,0)
        name=f'pi-{ident}-{child.pid}'
        run(['box','exec',name,'--','sh','-c','printf release >/tmp/review-gate'])
        assert queue.control(None,1,30),'native command did not complete'
        queue.close()
        if send_signal:os.kill(child.pid,signal.SIGTERM)
        result=child.wait(timeout=30)
        emit(f'{"F3" if send_signal else "F5"} native exit={code} observed through kqueue; caller output undrained; client signal={send_signal} result={result}')
        assert result==code
    finally:
        os.close(read)
        if child.poll() is None:child.terminate();child.wait(timeout=30)
        child.stderr.close()
for code in (0,23):completed(code)
completed(23,False)

name='probe-tail-'+str(os.getpid())
def owner():
    c=sp.Popen([binary,'box','up'],env=env,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
    c.stdin.write(json.dumps(dict(name=name,image=image,cpus=2,memory='1G')).encode()+b'\n');c.stdin.flush()
    assert json.loads(c.stdout.readline())['event']=='ready'
    return c

def write_console(text):
    return run(['box','exec',name,'--','sh','-c',text])
# Native-client death is observed before readiness, without a timing sleep.
child=sp.Popen([binary,'box','up'],env=env,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
child.stdin.write(json.dumps(dict(name=name,image=image,cpus=2,memory='1G')).encode()+b'\n');child.stdin.flush()
deadline=time.monotonic()+30;killed=False
while time.monotonic()<deadline and child.poll() is None:
    pids=(ctypes.c_int*64)();count=lib.proc_listchildpids(child.pid,pids,ctypes.sizeof(pids))
    for pid in pids[:count]:
        if not pid:continue
        try:args=sp.check_output(['ps','-p',str(pid),'-o','args=']).decode()
        except sp.CalledProcessError:continue
        if ' run -i ' in args and '--name '+name+' ' in args:
            assert not select.select([child.stdout],[],[],0)[0],'startup window already passed'
            os.kill(pid,signal.SIGKILL);killed=True;break
    if killed:break
    os.sched_yield()
assert killed,'native run spawn barrier absent'
out,err=child.communicate(timeout=15)
assert child.returncode==1 and json.loads(out)['event']=='failed'
emit('G1 native run killed at observed spawn before ready; startup failed and cleanup completed without hanging')
run(['box','down',name])

# Observe real private-PTY state and exercise control bytes without translation.
child=owner()
for code in (0,23):
    result=run(['box','exec',name,'--tty','--','sh','-c',f'exit {code}'],data=b'x'*1048576,code=code)
    emit(f'F3 forced-TTY pending1MiB input and immediate native exit: observed status={result.returncode}')
master,slave=os.openpty(); caller=os.ttyname(slave)
driver=os.fork()
if driver==0:
    os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
    for fd in range(3):os.dup2(slave,fd)
    os.tcsetpgrp(0,os.getpgrp());os.close(master);os.close(slave)
    command="stty raw -echo; exec perl -e '$|=1; print \"RAW-READY\\n\"; read(STDIN,my $s,6); print unpack(\"H*\",$s),\"\\n\";'"
    target=sp.Popen([binary,'box','exec',name,'--tty','--','sh','-c',command],env=env)
    os.write(1,f'PINPID={target.pid}\n'.encode());result=target.wait()
    os.write(1,f'PTY-RESULT={result}\n'.encode());os._exit(0)
os.close(slave);output=b''
try:
    while b'RAW-READY' not in output:
        assert select.select([master],[],[],60)[0]
        output+=os.read(master,65536)
    pin=int(re.search(rb'PINPID=([0-9]+)',output)[1]);native=exec_pid(pin)
    observer=sp.run(['/tmp/pfrt-review/fd-inspect',str(pin),str(native),caller],capture_output=True)
    assert observer.returncode==0,(observer.returncode,observer.stdout,observer.stderr)
    emit(observer.stdout.decode().strip())
    os.write(master,b'A\r\x03\0ZQ')
    while b'PTY-RESULT=' not in output:
        assert select.select([master],[],[],30)[0]
        output+=os.read(master,65536)
    assert b'410d03005a51' in output and b'PTY-RESULT=0' in output
    emit('F1 real forced-TTY wire retained CR, INTR and NUL exactly; guest exit0')
finally:
    os.close(master);os.waitpid(driver,0)
    run(['box','down',name]);child.communicate(timeout=30)

child=owner()
try:
    write_console("head -c 98304 /dev/zero | tr '\\000' X >/proc/1/fd/2; printf '\\nTAIL-END\\033[31m' >/proc/1/fd/2")
    sp.run(['container','kill','--signal','KILL',name],check=True,capture_output=True)
    out,err=child.communicate(timeout=30)
    assert child.returncode==1 and b'TAIL-END' in err and b'\\n' in err and b'\\u{1b}' in err
    assert b'\x1b' not in err and err.count(b'X')<=65536
    emit('G2/G3 guest console stderr 96KiB completed; unexpected exit reports escaped last64KiB including TAIL-END')
finally:
    run(['box','down',name])
child=owner()
try:
    write_console("printf NORMAL-TAIL >/proc/1/fd/2")
    run(['box','down',name]);out,err=child.communicate(timeout=30)
    assert child.returncode==0 and b'NORMAL-TAIL' not in err
    emit('G3 same console stderr on requested down is not printed; owner exit=0')
finally:
    if child.poll() is None:run(['box','down',name]);child.communicate(timeout=30)
run(['disk','rm','--all','--json'])
emit('PASS selected real Apple review cases; fixture disks removed')
