#!/usr/bin/env python3
"""Real command cleanup after a guest helper poll failure: cleanup.py BIN IMAGE."""
import hashlib,json,os,pathlib,select,signal,subprocess as sp,sys,tempfile,time
B=str(pathlib.Path(sys.argv[1]).resolve()); IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfc.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch(); ident='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
PROGRAM=r'''use strict; use warnings; use POSIX qw(setsid uname pause); use IO::Poll qw(POLLERR POLLHUP);
$|=1; my $mode=shift; my $helper=getppid(); my $sys=(uname())[4] eq 'aarch64' ? 261 : 302;
my $old=pack('Q2',0,0); syscall($sys,$helper,7,0,$old)==0 or die "query prlimit: $!";
my($soft,$hard)=unpack('Q2',$old); my $restorer=0;
$SIG{PIPE}='IGNORE'; $SIG{TERM}='IGNORE'; $SIG{HUP}='IGNORE';
if($mode eq 'fault') {
 pipe(my $r,my $w) or die $!; $restorer=fork(); die $! unless defined $restorer;
 if(!$restorer) {
  close $r; setsid()>=0 or die $!; print $w "RESTORER_READY\n"; close $w;
  my $p=IO::Poll->new(); $p->mask(\*STDOUT=>POLLERR|POLLHUP); $p->poll(15);
  my $events=$p->events(\*STDOUT); my $rc=syscall($sys,$helper,7,$old,0); my $error=0+$!;
  open my $f,'>','/tmp/probe-cleanup-restored' or die $!; print $f "events=$events rc=$rc errno=$error\n"; close $f; exit 0;
 }
 close $w; scalar <$r>; close $r;
}
print "READY:$$:$helper:$restorer\n"; scalar <STDIN>;
if($mode eq 'fault') { my $limit=pack('Q2',1,$hard); syscall($sys,$helper,7,$limit,0)==0 or die "set prlimit: $!"; print STDERR "POLL_FAULT_ARMED\n"; }
else { print "CONTROL_ACK\n"; }
pause() while 1;
'''
(P/'cleanup.pl').write_text(PROGRAM)
sp.run(['git','init','-q',str(P)],check=True);sp.run(['git','-C',str(P),'add','.'],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','-qm','fixture'],check=True)
def emit(x):print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,check=True):
 r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=25)
 if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
run(['config','edit']);C=next((W/'c/pinfold/projects').glob('*/pinfold.toml'));C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
children=[]; NAME=None
def line(p):
 assert select.select([p.stdout],[],[],20)[0],'guest readiness deadline'
 data=p.stdout.readline();assert data,'guest ended before readiness';return data
try:
 run(['start']); marker=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project');NAME=json.loads(marker.read_text())['name']
 def box(cmd):return run(['box','exec',NAME,'--','sh','-c',cmd]).stdout.decode().strip()
 def state(pid):return box(f'cat /proc/{pid}/stat 2>/dev/null || true').rsplit(')',1)[-1].strip()[:1]
 for mode in ['control','fault']:
  p=sp.Popen([B,'--','perl','cleanup.pl',mode],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);children.append(p)
  ready=line(p);assert ready.startswith(b'READY:'),ready;pid,helper,restorer=map(int,ready.strip().split(b':')[1:]);p.stdin.write(b'go\n');p.stdin.flush()
  if mode=='control':
   assert line(p)==b'CONTROL_ACK\n';assert state(pid) not in ('','Z','X') and p.poll() is None
   emit('control: same-uid prlimit query succeeds; command remains alive without limit reduction')
   p.send_signal(signal.SIGTERM);assert p.wait(timeout=8)==143
   end=time.monotonic()+6
   while state(pid) not in ('','Z','X'):assert time.monotonic()<end,'control departure cleanup deadline'
  else:
   started=time.monotonic();code=p.wait(timeout=8);detail=p.stderr.read();assert code==125,(code,detail);emit('fault diagnostic: '+repr(detail));end=time.monotonic()+6
   restored=box('cat /tmp/probe-cleanup-restored 2>/dev/null || true')
   while not restored:
    assert time.monotonic()<end,'restorer deadline';restored=box('cat /tmp/probe-cleanup-restored 2>/dev/null || true')
   assert state(pid) in ('','Z','X'),('unsupervised command survived',pid)
   assert 'rc=0' in restored or ('errno=3' in restored and sys.platform=='darwin'),restored
   emit(f'fault: helper RLIMIT_NOFILE=1 forces poll EINVAL; client=125; command gone; {restored}; elapsed={time.monotonic()-started:.2f}s')
 emit('RELAY FAILURE CLEANUP AND CONTROL PASSED')
finally:
 for p in children:
  if p.poll() is None:p.kill();p.wait(timeout=10)
 run(['stop'])
 for suffix in ['home','project']:run(['disk','rm','pi-'+ident+'-'+suffix])
 emit('cleanup: own box and two explicitly named disks removed')
