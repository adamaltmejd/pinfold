#!/usr/bin/env python3
"""Real-binary foreground handoff: SCRIPT BIN IMAGE."""
import fcntl, hashlib, json, os, select, signal, struct, subprocess as sp, sys, tempfile, termios, time
from pathlib import Path
binary = str(Path(sys.argv[1]).resolve())
work = Path(tempfile.mkdtemp(prefix="pffg.", dir="/tmp")).resolve()
root = work / "probe-foreground"
root.mkdir()
sp.run(["git", "init", "-q", str(root)], check=True)
env = {k: v for k, v in os.environ.items() if not k.startswith("PINFOLD_")}
env.update(XDG_CONFIG_HOME=str(work/"c"), XDG_STATE_HOME=str(work/"s"), PINFOLD_NO_UPDATE_CHECK="1", PINFOLD_CPUS="2", PINFOLD_MEMORY="1G")
(work/"s/pinfold").mkdir(parents=True)
(work/"s/pinfold/maintenance").touch()
ident = root.name + "-" + hashlib.sha256(os.fsencode(root)).hexdigest()[:12]
cfg = work/"c/pinfold/projects"/ident/"pinfold.toml"
cfg.parent.mkdir(parents=True)
cfg.write_text('mounts = [{ host = ".", kind = "disk" }]\nimage = '+json.dumps(sys.argv[2])+'\n')
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH",36,120,0,0))
control, command = os.pipe()
result, report = os.pipe()
helper_input, helper_command = os.pipe()
ack, acknowledged = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(master); os.close(command); os.close(result)
    os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY,0)
    for fd in range(3): os.dup2(slave,fd)
    os.close(slave); os.tcsetpgrp(0,os.getpgrp())
    signal.signal(signal.SIGTTIN,signal.SIG_IGN)
    signal.signal(signal.SIGTTOU,signal.SIG_DFL)
    owner = os.getpgrp()
    helper = os.fork()
    if helper == 0:
        os.setpgid(0,0); os.close(helper_command); os.close(ack)
        if os.read(helper_input,1) != b"H": os._exit(2)
        mode = termios.tcgetattr(0)
        mode[3] = (mode[3]|termios.ICANON|termios.ISIG)&~(termios.ECHO|termios.TOSTOP)
        termios.tcsetattr(0,termios.TCSANOW,mode)
        expected = termios.tcgetattr(0)
        expected[6] = [c if isinstance(c,int) else c[0] for c in expected[6]]
        os.write(acknowledged,json.dumps(expected).encode()+b"\n")
        if os.read(helper_input,1) == b"R": os.tcsetpgrp(0,owner)
        os._exit(0)
    os.setpgid(helper,helper)
    os.close(helper_input); os.close(acknowledged)
    child = sp.Popen([binary,"--","sh","-c","echo READY; read line; exec tail -f /dev/null"],cwd=root,env=env)
    assert os.read(control,1) == b"H"
    os.tcsetpgrp(0,helper); os.write(helper_command,b"H")
    expected = json.loads(os.fdopen(ack,"rb").readline())
    os.kill(child.pid,signal.SIGTERM)
    code = child.wait()
    observed = termios.tcgetattr(0)
    observed[6] = [c if isinstance(c,int) else c[0] for c in observed[6]]
    os.write(report, f"expected={expected!r} observed={observed!r}\n".encode())
    unchanged = observed == expected
    os.write(helper_command,b"R"); os.waitpid(helper,0)
    os.write(report,f"exit={code} foreground-helper-settings-untouched={unchanged}\n".encode())
    os._exit(0 if code == 143 and unchanged else 1)
os.close(slave); os.close(control); os.close(report); os.close(helper_input); os.close(acknowledged); os.close(ack)
output, final, handed = b"", b"", False
deadline = time.monotonic()+60
try:
    while time.monotonic()<deadline:
        ready,_,_ = select.select([master,result],[],[],0.2)
        if master in ready: output += os.read(master,65536)
        if not handed and b"READY\n" in output.replace(b"\r",b""):
            os.write(command,b"H"); handed=True
        if result in ready:
            final += os.read(result,65536)
            if b"foreground-helper-settings-untouched=" in final: break
    else: raise RuntimeError("handoff deadline; foreground owner may have stopped")
    _, status = os.waitpid(pid,0)
    print("$ $BIN -- sh -c 'echo READY; read line; exec tail -f /dev/null'; hand foreground to helper; helper stty canonical -echo; SIGTERM pinfold",flush=True)
    print(final.decode().strip()+"; host-SIGTTOU=default; handed="+str(handed),flush=True)
    assert os.waitstatus_to_exitcode(status)==0 and handed
finally:
    if not final:
        try: os.write(helper_command,b"R")
        except BrokenPipeError: pass
        os.killpg(pid,signal.SIGCONT); os.killpg(pid,signal.SIGTERM)
        os.waitpid(pid,0)
    for fd in [master,result,command,helper_command]: os.close(fd)
    removed=sp.run([binary,"disk","rm","--all","--json"],env=env,capture_output=True)
    print("cleanup-exit="+str(removed.returncode)+" fixture="+str(work),flush=True)
