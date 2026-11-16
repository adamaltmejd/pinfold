#!/usr/bin/env python3
"""Real command completion under a slow output consumer: BIN IMAGE [baseline]."""
import fcntl,hashlib,json,os,pathlib,select,signal,subprocess as sp,sys,tempfile,time,threading
B=str(pathlib.Path(sys.argv[1]).resolve()); IMAGE=sys.argv[2]; baseline=len(sys.argv)>3
W=pathlib.Path(tempfile.mkdtemp(prefix='pfsr.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}
E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
ident='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]; children=[]; peer=None
PAYLOAD=1048576+16384
# Both inherited output pipes stay silent after the tail; stderr must not rescue EXIT.
PROGRAM='$|=1; my $gate="/tmp/slow-output-gate"; my $ready="/tmp/slow-output-ready"; unlink $gate; unlink $ready; system("mkfifo",$gate)==0 or die $!; my $p=fork(); die $! unless defined $p; if(!$p){close STDIN; POSIX::pause() while 1} my $n=0; while($n<1048576){my $r=sysread(STDIN,my $b,16384); die $! unless $r; print $b; $n+=$r} open my $f,">",$ready or die $!; close $f; open my $r,"<",$gate or die $!; scalar <$r>; close $r; print "x"x16384; exit 23'
# The release acknowledgement follows bounded CPU work, while the caller reads no output.
PEER='$|=1; print "PEER_READY\\n"; while(<STDIN>){my $sum=0; if(/^release/){my $fast=/^release_fast/; open my $w,">","/tmp/slow-output-gate" or die $!; print $w "go\\n"; close $w; if(!$fast){$sum^=$_ for 1..100000000} print "RELEASED\\n"}else{$sum^=$_ for 1..20000; print((-e "/tmp/slow-output-ready") ? "ACK_READY\\n" : "ACK\\n")}}'

def emit(text):print(str(text).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,check=True):
 r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=100)
 if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
def line(fd,expected=None):
 result=b'';end=time.monotonic()+15
 while b'\n' not in result:
  assert select.select([fd],[],[],max(0,end-time.monotonic()))[0],'peer readiness deadline'
  chunk=os.read(fd,1);assert chunk,'peer EOF';result+=chunk
 if expected is not None:assert result==expected,result
 return result
sp.run(['git','init','-q',str(P)],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);config=next((W/'c/pinfold/projects').glob('*/pinfold.toml'));config.write_text(config.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
try:
 run(['start']);emit('mode='+('baseline' if baseline else 'candidate')+' image='+IMAGE)
 peer=sp.Popen([B,'--','perl','-e',PEER],cwd=P,env=E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE);children.append(peer);line(peer.stdout.fileno(),b'PEER_READY\n')
 for slow in [False,True]:
  read,write=os.pipe();pipe_size='platform-default'
  if hasattr(fcntl,'F_SETPIPE_SZ'):pipe_size=fcntl.fcntl(read,fcntl.F_SETPIPE_SZ,4096)
  p=sp.Popen([B,'--','perl','-MPOSIX','-e',PROGRAM],cwd=P,env=E,stdin=sp.PIPE,stdout=write,stderr=sp.PIPE);children.append(p);os.close(write)
  feed_errors=[]
  def feed():
   try:p.stdin.write(b'x'*1048576);p.stdin.close()
   except (OSError,ValueError) as error:feed_errors.append(repr(error))
  feeder=threading.Thread(target=feed,daemon=True);feeder.start()
  open_fds={read:'stdout',p.stderr.fileno():'stderr'};total=0;errors=b'';barriers=0;t=time.monotonic();deadline=t+45;timed_out=False;released=False;release_sent=False;consumer_hold=0.0
  try:
   while open_fds:
    ready,_,_=select.select(list(open_fds),[],[],max(0,deadline-time.monotonic()))
    if not ready:timed_out=True;break
    for fd in ready:
     data=os.read(fd,4096 if slow else 65536)
     if not data:del open_fds[fd];continue
     if fd==read:
      assert data==b'x'*len(data),'unexpected command bytes';total+=len(data)
      if not slow and not release_sent:
       peer.stdin.write(b'release_fast\n');peer.stdin.flush();release_sent=True
      if slow:
       peer.stdin.write(b'ping\n');peer.stdin.flush();ack=line(peer.stdout.fileno());assert ack in (b'ACK\n',b'ACK_READY\n');barriers+=1
       if ack==b'ACK_READY\n' and not released:
        held=time.monotonic();peer.stdin.write(b'release\n');peer.stdin.flush();line(peer.stdout.fileno(),b'RELEASED\n');consumer_hold=time.monotonic()-held;released=True;release_sent=True
     else:errors+=data
   if not slow and release_sent:
    line(peer.stdout.fileno(),b'RELEASED\n');released=True
   if timed_out:code=None
   else:
    try:code=p.wait(timeout=5)
    except sp.TimeoutExpired:code=None;timed_out=True
   emit(f'{"slow peer-barrier reader" if slow else "continuous reader control"}: status={code}, bytes={total}, peer_acks={barriers}, consumer_hold={consumer_hold:.2f}s, pipe_size={pipe_size}, stdin_bytes=1048576, elapsed={time.monotonic()-t:.2f}s'+(' HANG' if timed_out else ''))
   if not baseline:
    feeder.join(timeout=5);assert not feeder.is_alive() and not feed_errors,feed_errors
    assert code==23,(code,errors)
    assert total>=65536,total
    if not slow:assert total==PAYLOAD and released,(total,released)
    if slow:assert barriers>=16 and released,(barriers,released)
   if code is None:p.send_signal(signal.SIGTERM);p.wait(timeout=10)
  finally:os.close(read)
 emit('SLOW READER CASES COMPLETE')
finally:
 for p in children:
  if p.poll() is None:p.kill();p.wait(timeout=10)
 run(['stop'])
 for suffix in ['home','project']:run(['disk','rm','pi-'+ident+'-'+suffix])
 emit('cleanup: own kept box and two named disks removed')
