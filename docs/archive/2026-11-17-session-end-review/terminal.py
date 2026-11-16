#!/usr/bin/env python3
"""Manual real-binary terminal cases. Usage: terminal.py BINARY IMAGE."""
import fcntl, hashlib, json, os, pathlib, pty, re, select, signal, struct
import subprocess as sp
import sys, tempfile, termios, time

B = str(pathlib.Path(sys.argv[1]).resolve()); IMAGE = sys.argv[2]
W = pathlib.Path(tempfile.mkdtemp(prefix='pfr.', dir='/tmp')).resolve(); P = W/'p'; P.mkdir()
E = {k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CONFIG_HOME=str(W/'c'), XDG_STATE_HOME=str(W/'s'), PINFOLD_NO_UPDATE_CHECK='1',
         PINFOLD_CPUS='2', PINFOLD_MEMORY='1G', EDITOR='true', VISUAL='true', TERM='xterm-256color')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
PROJECT = 'p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
SCREEN = r'''use strict; use warnings; use POSIX qw(pause);
$|=1; my $go=0; $SIG{USR1}=sub{$go=1};
sub size { my $s=pack('S4',0,0,0,0); ioctl(STDIN,0x5413,$s) or die $!; my($r,$c)=unpack('S4',$s); print "SCREEN_SIZE:$r:$c\n"; }
$SIG{WINCH}=sub{size()}; system('stty','-icanon','-echo','min','1','time','0')==0 or die;
print "\e[?1049h\e[2J\e[HSCREEN_READY:$$\n"; size(); pause() until $go;
my($n,$bad)=(0,0); while($n<393216) { my $r=sysread(STDIN,my $b,393216-$n); next if !defined($r) && $!{EINTR}; die unless $r; $n+=$r; $bad+=($b=~tr/x/x/c); }
print "PASTE_OK:$n:$bad\n"; system('stty','icanon','echo')==0 or die; print "\e[?1049l";
'''
(P/'screen.pl').write_text(SCREEN)
sp.run(['git','init','-q',str(P)],check=True); sp.run(['git','-C',str(P),'add','.'],check=True)
sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','-qm','fixture'],check=True)

def emit(x):
    print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args, check=True):
    r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=90)
    if check and r.returncode: raise RuntimeError((args,r.returncode,r.stderr.decode()))
    return r
run(['config','edit']); C=next((W/'c/pinfold/projects').glob('*/pinfold.toml'))
C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
NAME=None; terminals=[]; peer_process=None

def box(command): return run(['box','exec',NAME,'--','sh','-c',command]).stdout.decode().strip()
def state(pid):
    text=box(f'cat /proc/{pid}/stat 2>/dev/null || true')
    return text.rsplit(')',1)[-1].strip()[:1]
def wait_state(pid,predicate,seconds=5):
    end=time.monotonic()+seconds
    while True:
        s=state(pid)
        if predicate(s): return s
        if time.monotonic()>end: raise RuntimeError(('process state deadline',pid,s))
def read_until(fd,needle,seconds=20):
    data=b''; end=time.monotonic()+seconds
    while needle not in data:
        if not select.select([fd],[],[],max(0,end-time.monotonic()))[0]: raise RuntimeError(('readiness deadline',needle,data[-300:]))
        chunk=os.read(fd,4096)
        if not chunk: raise RuntimeError(('unexpected EOF',needle,data[-300:]))
        data+=chunk
    return data

def peer():
    peer_process.stdin.write(b'ping\n'); peer_process.stdin.flush()
    read_until(peer_process.stdout.fileno(),b'PEER_OK')

class Terminal:
    def __init__(self,argv,direct=False):
        self.master,self.slave=pty.openpty(); self.data=b''; self.report=b''; self.result=None
        fcntl.ioctl(self.slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
        read,write=os.pipe(); self.report_fd=read
        self.direct=None
        if direct:
            def setup():
                os.setsid(); fcntl.ioctl(self.slave,termios.TIOCSCTTY,0)
                signal.signal(signal.SIGHUP,signal.SIG_DFL)
            self.direct=sp.Popen([B,'--']+argv,cwd=P,env=E,stdin=self.slave,stdout=self.slave,stderr=self.slave,preexec_fn=setup)
            os.close(read); os.close(write); self.report_fd=None; self.driver=None; self.client=self.direct.pid
            os.set_blocking(self.master,False); terminals.append(self); return
        self.driver=os.fork()
        if not self.driver:
            os.close(read); os.setsid(); fcntl.ioctl(self.slave,termios.TIOCSCTTY,0)
            for fd in range(3): os.dup2(self.slave,fd)
            os.close(self.master); os.close(self.slave); os.chdir(P)
            signal.signal(signal.SIGHUP,signal.SIG_IGN)
            before=termios.tcgetattr(0)
            child=sp.Popen([B,'--']+argv,env=E)
            def hangup(signum,frame):
                try: os.kill(child.pid,signal.SIGHUP)
                except ProcessLookupError: pass
            signal.signal(signal.SIGHUP,hangup)
            os.write(write,(json.dumps({'client':child.pid})+'\n').encode())
            code=child.wait()
            try:
                # A read-readiness query processes Darwin's kernel-managed PENDIN state.
                fcntl.ioctl(0,termios.FIONREAD,struct.pack('I',0))
                after=termios.tcgetattr(0); restored=after==before
                detail={'before':repr(before),'after':repr(after),'foreground':os.tcgetpgrp(0),'group':os.getpgrp()}
            except termios.error: restored=None; detail={}
            os.write(write,(json.dumps({'code':code,'restored':restored, 'detail':detail})+'\n').encode())
            os.close(write); os._exit(0)
        os.close(write); os.set_blocking(self.master,False); os.set_blocking(read,False)
        terminals.append(self); self.client=None
    def poll(self,timeout=0.1,write=False):
        fds=([self.report_fd] if self.report_fd is not None else [])+([self.master] if self.master is not None else [])
        ready,writable,_=select.select(fds,[self.master] if write and self.master is not None else [],[],timeout)
        for fd in ready:
            try: chunk=os.read(fd,65536)
            except (BlockingIOError,OSError): chunk=b''
            if fd==self.master: self.data+=chunk
            else:
                self.report+=chunk
                while b'\n' in self.report:
                    line,self.report=self.report.split(b'\n',1); item=json.loads(line)
                    if 'client' in item:self.client=item['client']
                    if 'code' in item:self.result=item
        return bool(writable)
    def expect(self,needle,start=0,seconds=20):
        end=time.monotonic()+seconds
        while needle not in self.data[start:]:
            if time.monotonic()>end: raise RuntimeError(('terminal marker deadline',needle,self.data[-300:]))
            self.poll()
        while self.client is None:
            if time.monotonic()>end:raise RuntimeError('client identity deadline')
            self.poll()
        return self.data[start:]
    def send(self,data):
        end=time.monotonic()+20; offset=0
        while offset<len(data):
            if time.monotonic()>end:raise RuntimeError('terminal send deadline')
            self.poll(write=True)
            try:offset+=os.write(self.master,data[offset:])
            except BlockingIOError:pass
    def command(self,text,needle):
        start=len(self.data); self.send(text.encode()+b'\n'); return self.expect(needle,start)
    def resize(self,rows,cols):
        fcntl.ioctl(self.master,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
        if self.client:os.kill(self.client,signal.SIGWINCH)
    def finish(self,code,restore=True):
        if self.direct is not None:
            assert not restore
            assert self.direct.wait(timeout=10)==code
            return {'code':code,'restored':None}
        end=time.monotonic()+15
        while self.result is None:
            if time.monotonic()>end:raise RuntimeError(('terminal exit deadline',self.data[-300:]))
            self.poll()
        assert self.result['code']==code,self.result
        if restore:assert self.result['restored'] is True,self.result
        os.waitpid(self.driver,0); self.driver=None
        return self.result
    def close(self):
        if self.master is not None:os.close(self.master); self.master=None
    def cleanup(self):
        self.close()
        if self.direct is not None and self.direct.poll() is None:
            self.direct.kill(); self.direct.wait(timeout=10)
        if self.driver:
            if self.client:
                try:os.kill(self.client,signal.SIGKILL)
                except ProcessLookupError:pass
            try:os.kill(self.driver,signal.SIGTERM)
            except ProcessLookupError:pass
            os.waitpid(self.driver,0); self.driver=None
        os.close(self.slave)
        if self.report_fd is not None: os.close(self.report_fd)

def bash(hist='/tmp/probe-shell-history',direct=False):
    t=Terminal(['env','PS1=PF_PROMPT> ',f'HISTFILE={hist}','HISTSIZE=100','HISTFILESIZE=100','HISTCONTROL=',
                'bash','--noprofile','--norc','-i'],direct=direct)
    t.expect(b'PF_PROMPT> '); t.command('ulimit -c 0; printf \'SHELL_\'\'READY\\n\'',b'SHELL_READY'); return t

try:
    run(['start'])
    marker=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project')
    NAME=json.loads(marker.read_text())['name']
    peer_process=sp.Popen([B,'--','perl','-e','$|=1; print "PEER_READY\\n"; while(<STDIN>){print "PEER_OK\\n"}'],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
    read_until(peer_process.stdout.fileno(),b'PEER_READY')
    t=Terminal(['perl','screen.pl']); data=t.expect(b'SCREEN_SIZE:24:80')
    guest=int(re.search(rb'SCREEN_READY:(\d+)',data).group(1))
    start=len(t.data); t.resize(31,101); t.expect(b'SCREEN_SIZE:31:101',start)
    pending=b'x'*393216; sent=0; end=time.monotonic()+15
    while sent<len(pending):
        if time.monotonic()>end:raise RuntimeError('paste failed to saturate input')
        t.poll(write=True)
        try: sent+=os.write(t.master,pending[sent:])
        except BlockingIOError:
            if sent>=65536:break
    assert sent<len(pending),('expected unread-input saturation',sent)
    start=len(t.data); t.resize(43,137); t.expect(b'SCREEN_SIZE:43:137',start)
    box(f'kill -USR1 {guest}'); t.send(pending[sent:]); t.expect(b'PASTE_OK:393216:0')
    t.finish(0); peer(); emit(f'fullscreen resize: 24x80 -> 31x101 -> 43x137 during blocked 384KiB paste; exact input; termios restored; B=OK')
    t=bash(); data=t.command("sh -c 'printf \"QUIT_\"\"READY:%s\\n\" \"$$\"; exec sleep 600'",b'QUIT_READY:')
    q=int(re.search(rb'QUIT_READY:(\d+)',data).group(1)); assert state(q) not in ('','Z','X')
    start=len(t.data); t.send(b'\x1c'); t.expect(b'PF_PROMPT> ',start)
    data=t.command("printf 'QUIT_''STATUS:%s\\n' \"$?\"",b'QUIT_STATUS:131'); peer()
    data=t.command("sh -c 'printf \"STOP_\"\"READY:%s\\n\" \"$$\"; exec sleep 600'",b'STOP_READY:')
    stopped=int(re.search(rb'STOP_READY:(\d+)',data).group(1)); start=len(t.data); t.send(b'\x1a'); t.expect(b'PF_PROMPT> ',start)
    assert wait_state(stopped,lambda s:s=='T')=='T'; t.send(b'fg\n'); wait_state(stopped,lambda s:s not in ('T',''))
    start=len(t.data); t.send(b'\x03'); t.expect(b'PF_PROMPT> ',start)
    t.command("printf 'FG_''STATUS:%s\\n' \"$?\"",b'FG_STATUS:130'); t.send(b'\x04'); t.finish(0); peer()
    emit('Ctrl-\\: foreground status 131; Ctrl-Z stopped job; fg resumed it; Ctrl-C status 130; Ctrl-D shell exit 0; termios restored; B=OK')
    hist='/tmp/probe-hup-history'; t=bash(hist,direct=True)
    t.command("history -c; printf 'HISTORY_''CLEARED\\n'",b'HISTORY_CLEARED')
    data=t.command(": HISTORY_MARKER; sleep 600 & bg=$!; sleep 600 & st=$!; kill -STOP \"$st\"; printf 'JOBS_''READY:%s:%s:%s\\n' \"$$\" \"$bg\" \"$st\"",b'JOBS_READY:')
    shell,bg,st=map(int,re.search(rb'JOBS_READY:(\d+):(\d+):(\d+)',data).groups())
    assert state(shell) not in ('','Z','X') and state(bg) not in ('','Z','X'); wait_state(st,lambda s:s=='T')
    began=time.monotonic(); t.close(); t.finish(129,restore=False)
    for pid in [shell,bg,st]:wait_state(pid,lambda s:s in ('','Z','X'),seconds=1.5)
    elapsed=time.monotonic()-began; assert elapsed<1.8,elapsed
    assert 'HISTORY_MARKER' in box(f'cat {hist}'); peer()
    emit(f'terminal hangup: bash/background/stopped job gone in {elapsed:.3f}s; history marker saved; B=OK')
    cmd=['perl','-MPOSIX','-e','$|=1; print "RESTORE_READY\\n"; POSIX::pause() while 1']
    t=Terminal(cmd); t.expect(b'RESTORE_READY'); os.kill(t.client,signal.SIGTERM); t.finish(143); peer()
    emit('client SIGTERM: 143; caller termios restored; B=OK')
    t=Terminal(cmd); t.expect(b'RESTORE_READY'); run(['stop']); t.finish(125)
    assert peer_process.wait(timeout=10)==125
    emit('pinfold stop from another host process: incomplete client 125; caller termios restored')
    emit('ALL TERMINAL CASES PASSED')
finally:
    for t in terminals:t.cleanup()
    if peer_process is not None and peer_process.poll() is None:peer_process.kill(); peer_process.wait()
    run(['stop'])
    for suffix in ['home','project']:run(['disk','rm','pi-'+PROJECT+'-'+suffix])
    emit('cleanup: own box stopped; two explicitly named fixture disks removed')
