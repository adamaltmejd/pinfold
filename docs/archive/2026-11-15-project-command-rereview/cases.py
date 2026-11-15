#!/usr/bin/env python3
"""Real re-review cases: BIN IMAGE baseline|fixed [fresh-root]."""
import fcntl, hashlib, json, os, select, signal, subprocess as sp, sys, tempfile, termios, time
from pathlib import Path
binary = str(Path(sys.argv[1]).resolve())
image, phase = sys.argv[2:4]
fixed = phase == 'fixed'
work = Path(sys.argv[4] if len(sys.argv) > 4 else tempfile.mkdtemp(prefix='pfrr.', dir='/tmp')).resolve()
env = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
env.update(XDG_CONFIG_HOME=str(work/'c'), XDG_STATE_HOME=str(work/'s'), PINFOLD_NO_UPDATE_CHECK='1', PINFOLD_CPUS='2', PINFOLD_MEMORY='1G')
(work/'s/pinfold').mkdir(parents=True)
(work/'s/pinfold/maintenance').touch()
root = work/'probe-rereview'
root.mkdir()
(root/'execute').mkdir()
(root/'execute/fixture').write_text('committed fixture\n')
for args in [['init','-q'],['add','.'],['-c','user.name=user','-c','user.email=user@example.invalid','commit','-qm','fixture']]:
    sp.run(['git','-C',str(root)]+args,check=True,capture_output=True)
ident = root.name+'-'+hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg = work/'c/pinfold/projects'/ident/'pinfold.toml'
cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = '+json.dumps(image)+'\n')
def emit(value):
    print(value.replace(binary,'$BIN').replace(str(work),'$W'),flush=True)
def run(args,cwd=root,expected=0):
    result=sp.run([binary]+args,cwd=cwd,env=env,capture_output=True,timeout=120)
    emit(f'$ $BIN {args!r}; exit={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}')
    assert result.returncode==expected
    return result

def redirected_stty():
    master,slave=os.openpty()
    gate,release=os.pipe()
    pid=os.fork()
    if pid==0:
        os.close(release)
        os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        for fd in range(3): os.dup2(slave,fd)
        os.tcsetpgrp(0,os.getpgrp());os.close(master);os.close(slave);os.chdir(root)
        signal.signal(signal.SIGTTIN,signal.SIG_IGN)
        child=sp.Popen([binary,'--','sh','-c','mkfifo gate; echo CMD-READY; cat gate; rm gate'],env=env,stdout=sp.PIPE,stderr=sp.PIPE)
        assert child.stdout.readline()==b'CMD-READY\n'
        os.write(1,b'DRIVER-CHANGE-STTY\n')
        os.read(gate,1);os.close(gate)
        name=f'pi-{ident}-{child.pid}'
        released=sp.run([binary,'box','exec',name,'--','sh','-c','printf "CMD-DONE\n" > "$1"','sh',str(root/'gate')],env=env,stdin=sp.DEVNULL,capture_output=True,timeout=60)
        code=child.wait()
        out=child.stdout.read();err=child.stderr.read()
        settings=sp.run(['stty','-g'],capture_output=True,check=True).stdout.strip()
        retained=not bool(termios.tcgetattr(0)[3]&termios.ECHO)
        os.write(1,f'DRIVER-RESULT code={code} changed-setting-retained={retained} stty={settings!r} stdout={out!r} stderr={err!r}\n'.encode())
        os._exit(0)
    os.close(gate)
    output=b'';changed=False;deadline=time.monotonic()+120
    try:
        while time.monotonic()<deadline:
            if select.select([master],[],[],0.2)[0]:
                try: chunk=os.read(master,65536)
                except OSError: chunk=b''
                output+=chunk
            if b'DRIVER-CHANGE-STTY' in output and not changed:
                attrs=termios.tcgetattr(slave);attrs[3]&=~termios.ECHO
                termios.tcsetattr(slave,termios.TCSANOW,attrs)
                os.write(release,b'x');os.close(release);changed=True
            if os.waitpid(pid,os.WNOHANG)[0]:
                while select.select([master],[],[],0)[0]:
                    try: chunk=os.read(master,65536)
                    except OSError: break
                    if not chunk: break
                    output+=chunk
                break
        else:
            os.killpg(pid,signal.SIGTERM);os.waitpid(pid,0)
            raise RuntimeError('PTY driver deadline')
        emit('$ $BIN -- sh -c "mkfifo gate; echo CMD-READY; cat gate; rm gate"; stdin=PTY, stdout=pipe; ignore SIGTTIN; change host stty -echo after CMD-READY; release guest FIFO through public box exec')
        emit(f'phase={phase} output={output!r}')
        assert b'code=0' in output and b'CMD-DONE' in output and (b'changed-setting-retained=True' if fixed else b'changed-setting-retained=False') in output
    finally:
        if not changed: os.close(release)
        os.close(master);os.close(slave)
try:
    redirected_stty()
    run(['--','chmod','111','execute'])
    result=run(['--','sh','-c','pwd; echo ENTERED'],root/'execute',0 if fixed else 125)
    if fixed: assert result.stdout==(str(root/'execute')+'\nENTERED\n').encode()
    run(['--','sh','-c','chmod 755 execute; rm -rf execute; ln -s /home/pi execute'])
    result=run(['--','pwd'],root/'execute',125)
    assert b'working-directory' in result.stderr
    run(['--','sh','-c','rm execute; mkdir execute'])
    result=run(['--','pwd'],root/'execute')
    assert result.stdout==(str(root/'execute')+'\n').encode()
    emit('PASS non-raw terminal preservation, execute-only cwd, symlink refusal and plain cwd')
finally:
    cleanup=sp.run([binary,'disk','rm','--all','--json'],env=env,capture_output=True,timeout=120)
    emit(f'isolated disk cleanup exit={cleanup.returncode}')
    assert cleanup.returncode==0
