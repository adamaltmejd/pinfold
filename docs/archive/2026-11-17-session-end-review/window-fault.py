#!/usr/bin/env python3
"""Manual Darwin ioctl fault/control. Usage: window-fault.py BINARY IMAGE."""
import fcntl, hashlib, json, os, pathlib, pty, select, signal, struct
import subprocess as sp
import sys, tempfile, termios, time

if sys.platform!='darwin':raise SystemExit('This observer injects a Darwin ioctl failure.')
B=str(pathlib.Path(sys.argv[1]).resolve()); IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfw.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_') and k!='DYLD_INSERT_LIBRARIES'}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',
         PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true',TERM='xterm-256color')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
PROJECT='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]; ARM=W/'armed'
C=r'''#include <sys/ioctl.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <errno.h>
static int replacement(int fd, unsigned long request, ...) {
    void *arg = NULL;
    if (!(request & IOC_VOID)) { va_list ap; va_start(ap, request); arg=va_arg(ap,void *); va_end(ap); }
    const char *arm=getenv("PINFOLD_PROBE_WINDOW_ARM");
    if (request==TIOCGWINSZ && strcmp(getprogname(),"pinfold")==0 && arm && access(arm,F_OK)==0) {
        static const char marker[]="WINDOW_IOCTL_EIO\n";
        (void)write(2,marker,sizeof(marker)-1); errno=EIO; return -1;
    }
    return (request & IOC_VOID) ? ioctl(fd,request) : ioctl(fd,request,arg);
}
__attribute__((used,section("__DATA,__interpose")))
static const struct { const void *replacement; const void *original; } pair = {
    (const void *)replacement, (const void *)ioctl
};
'''
(W/'window.c').write_text(C)
sp.run(['clang','-dynamiclib','-Wall','-Wextra','-Werror',str(W/'window.c'),'-o',str(W/'window.dylib')],check=True)
INJECT=dict(E,DYLD_INSERT_LIBRARIES=str(W/'window.dylib'),PINFOLD_PROBE_WINDOW_ARM=str(ARM))
sp.run(['git','init','-q',str(P)],check=True)
sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','--allow-empty','-qm','fixture'],check=True)
def emit(x): print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/Users/user'),flush=True)
def run(args,check=True):
    r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=90)
    if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
    return r
run(['config','edit']); conf=next((W/'c/pinfold/projects').glob('*/pinfold.toml'))
conf.write_text(conf.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
PROGRAM='$|=1; $SIG{WINCH}=sub{my $s=pack("S4",0,0,0,0); ioctl(STDIN,0x5413,$s) or die $!; my($r,$c)=unpack("S4",$s); print "WINDOW_SIZE:$r:$c\\n"; exit 0}; print "WINDOW_READY\\n"; POSIX::pause() while 1'

def execute(fault):
    ARM.unlink(missing_ok=True); master,slave=pty.openpty(); read,write=os.pipe()
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
    driver=os.fork()
    if not driver:
        os.close(read); os.setsid(); fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        for fd in range(3):os.dup2(slave,fd)
        os.close(master); os.close(slave); os.chdir(P)
        before=termios.tcgetattr(0); child=sp.Popen([B,'--','perl','-MPOSIX','-e',PROGRAM],env=INJECT)
        os.write(write,(json.dumps({'client':child.pid})+'\n').encode()); code=child.wait()
        fcntl.ioctl(0,termios.FIONREAD,struct.pack('I',0))
        os.write(write,(json.dumps({'code':code,'restored':termios.tcgetattr(0)==before})+'\n').encode()); os.close(write); os._exit(0)
    os.close(write); os.set_blocking(master,False); os.set_blocking(read,False)
    data=b''; reports=b''; client=None; result=None; resized=False; end=time.monotonic()+25
    try:
        while result is None:
            if time.monotonic()>end:raise RuntimeError(('window case deadline',data[-300:]))
            ready,_,_=select.select([master,read],[],[],max(0,end-time.monotonic()))
            for fd in ready:
                try:chunk=os.read(fd,65536)
                except (BlockingIOError,OSError):chunk=b''
                if fd==master:data+=chunk
                else:
                    reports+=chunk
                    while b'\n' in reports:
                        line,reports=reports.split(b'\n',1); report=json.loads(line)
                        if 'client' in report:client=report['client']
                        if 'code' in report:result=report
            if not resized and client and b'WINDOW_READY' in data:
                if fault:ARM.touch()
                fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',41,133,0,0)); os.kill(client,signal.SIGWINCH); resized=True
        expected=125 if fault else 0
        assert result=={'code':expected,'restored':True},(result,data[-300:])
        assert resized
        assert (b'WINDOW_IOCTL_EIO' in data)==fault,data[-300:]
        if not fault:assert b'WINDOW_SIZE:41:133' in data,data[-300:]
        os.waitpid(driver,0); driver=None
        emit(('fault' if fault else 'control')+f': project status={expected}; window ioctl EIO={fault}; exact caller termios restored=True')
    finally:
        ARM.unlink(missing_ok=True); os.close(master); os.close(slave); os.close(read)
        if driver:
            if client:
                try:os.kill(client,signal.SIGKILL)
                except ProcessLookupError:pass
            try:os.kill(driver,signal.SIGTERM)
            except ProcessLookupError:pass
            os.waitpid(driver,0)
try:
    run(['start']); execute(False); execute(True); emit('WINDOW QUERY FAULT AND CONTROL PASSED')
finally:
    ARM.unlink(missing_ok=True); run(['stop'])
    for suffix in ['home','project']:run(['disk','rm','pi-'+PROJECT+'-'+suffix])
    emit('cleanup: own box stopped; two explicitly named fixture disks removed')
