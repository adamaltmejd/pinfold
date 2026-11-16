#!/usr/bin/env python3
"""Manual pipe cases: pipes.py BINARY IMAGE [baseline]."""
import hashlib,json,os,pathlib,select,signal,subprocess as s,sys,tempfile,time,threading
B=str(pathlib.Path(sys.argv[1]).resolve()); I=sys.argv[2]; baseline=len(sys.argv)>3
W=pathlib.Path(tempfile.mkdtemp(prefix='pfq.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}; E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch(); ident='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
def emit(x):print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,check=True):
 r=s.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=100)
 if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
s.run(['git','init','-q',str(P)],check=True);s.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','-qm','fixture','--allow-empty'],check=True)
run(['config','edit']); C=next((W/'c/pinfold/projects').glob('*/pinfold.toml')); C.write_text(C.read_text()+f'image={json.dumps(I)}\nallow=[]\n')
children=[]
def exact(fd,count):
 result=b'';end=time.monotonic()+8
 while len(result)<count:
  assert select.select([fd],[],[],max(0,end-time.monotonic()))[0],'stream deadline'
  chunk=os.read(fd,min(65536,count-len(result)));assert chunk,'early EOF';result+=chunk
 return result
def launch(argv,stdin=s.DEVNULL,**kw):
 p=s.Popen([B,'--']+argv,cwd=P,env=E,stdin=stdin,stdout=s.PIPE,stderr=s.PIPE,**kw);children.append(p);return p
try:
 run(['start']); emit('mode='+('main-baseline' if baseline else 'candidate')+' image='+I)
 for stream in ['stdout','stderr']:
  finite=run(['--','sh','-c','printf "CONTROL\\n"'+(' >&2' if stream=='stderr' else '')]);assert getattr(finite,stream)==b'CONTROL\n';emit(stream+' finite producer: status=0 bytes=CONTROL')
  p=launch(['sh','-c','exec yes'+(' >&2' if stream=='stderr' else '')]);consumer=getattr(p,stream)
  assert consumer.readline()==b'y\n';consumer.close();t=time.monotonic()
  try:code=p.wait(timeout=6);emit(stream+' yes | head -n 1: status='+str(code)+f' elapsed={time.monotonic()-t:.2f}s')
  except s.TimeoutExpired:
   emit(stream+' yes | head -n 1: HANG after 6s');p.send_signal(signal.SIGTERM);p.wait(timeout=10);code=None
  if not baseline:assert code==141,(stream,code)
 if not baseline:
  # The leader exits independently of a background writer. Do not read stdout.
  p=launch(['perl','-e','$|=1; if(fork()==0){$SIG{PIPE}="IGNORE"; while(1){print "x"x16384}} print STDERR "LEADER_EXIT\\n"; exit 23']); assert select.select([p.stderr],[],[],10)[0];assert p.stderr.readline()==b'LEADER_EXIT\n';t=time.monotonic();assert p.wait(timeout=4)==23;emit(f'unread stdout at leader exit: status=23 elapsed={time.monotonic()-t:.2f}s')
  control=run(['--','perl','-e','print "x"x1048576; exit 23'],False);assert control.returncode==23 and control.stdout==b'x'*1048576;emit('read output control: status=23 exact 1048576 bytes')
  p=launch(['perl','-e','$|=1; if(fork()==0){while(1){print "x"x16384}} print STDERR "b"x1048576; exit 23']); total=[0]
  def discard():
   while True:
    chunk=p.stdout.read(65536)
    if not chunk:return
    total[0]+=len(chunk)
  drain=threading.Thread(target=discard);drain.start();assert select.select([p.stderr],[],[],5)[0];assert exact(p.stderr.fileno(),1048576)==b'b'*1048576;assert p.wait(timeout=5)==23;drain.join(timeout=3);assert not drain.is_alive() and total[0]>0;emit('concurrent stdout/stderr: exact 1MiB stderr during continuous stdout; status=23')
  p=launch(['perl','-e','$|=1; if(fork()==0){while(1){print "x"x16384}} print STDERR "INPUT_READY\\n"; my $x=<STDIN>; print STDERR "INPUT_GOT:$x"; exit 23'],stdin=s.PIPE)
  drain=threading.Thread(target=discard);drain.start();assert p.stderr.readline()==b'INPUT_READY\n';p.stdin.write(b'MARKER\n');p.stdin.flush();assert select.select([p.stderr],[],[],5)[0];assert p.stderr.readline()==b'INPUT_GOT:MARKER\n';assert p.wait(timeout=5)==23;drain.join(timeout=3);assert not drain.is_alive();emit('continuous stdout: queued stdin reaches guest; status=23')
  for code in [0,23]:
   p=launch(['sh','-c',f'echo READY >&2; sleep 30 & exit {code}']);assert p.stderr.readline()==b'READY\n';r=run(['--','true']);p.send_signal(signal.SIGTERM);assert p.wait(timeout=5)==code;emit('completed status during later signal: '+str(code))
 emit('PIPE CASES PASSED')
finally:
 for p in children:
  if p.poll() is None:p.kill();p.wait(timeout=10)
 run(['stop'])
 for suffix in ['home','project']:run(['disk','rm','pi-'+ident+'-'+suffix])
 emit('cleanup: own box and two named disks removed')
