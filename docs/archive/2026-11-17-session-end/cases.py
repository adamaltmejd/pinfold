import hashlib,json,os,pathlib,pty,select,signal,subprocess as s,sys,tempfile,time
B=str(pathlib.Path(sys.argv[1]).resolve()); image=sys.argv[2]; D=pathlib.Path(__file__).resolve().parent
W=pathlib.Path(tempfile.mkdtemp(prefix='pfs.',dir='/tmp')).resolve(); P=W/'p'; P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')}; E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch(); project='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
def emit(x): print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,check=True):
 r=s.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=100)
 if check and r.returncode: raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
s.run(['git','init','-q',str(P)],check=True); (P/'workload.pl').write_bytes((D/'workload.pl').read_bytes()); s.run(['git','-C',str(P),'add','.'],check=True); s.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','-qm','fixture'],check=True)
run(['config','edit']); C=next((W/'c/pinfold/projects').glob('*/pinfold.toml')); C.write_text(C.read_text()+f'image={json.dumps(image)}\nallow=[]\n')
def line(fd,needle=b'READY',timeout=30):
 buf=b''; end=time.monotonic()+timeout
 while needle not in buf:
  if not select.select([fd],[],[],max(0,end-time.monotonic()))[0]: raise RuntimeError(('readiness',buf))
  try: data=os.read(fd,4096)
  except OSError: data=b''
  if not data: raise RuntimeError(('EOF',buf))
  buf+=data
 return buf
def wait_terminal(p,fd):
 end=time.monotonic()+10
 while p.poll() is None:
  if not select.select([fd],[],[],max(0,end-time.monotonic()))[0]: raise RuntimeError('terminal exit deadline')
  try:
   if not os.read(fd,4096): break
  except OSError: break
 return p.wait(timeout=5)
children=[]; masters=set()
def closepty(fd):
 os.close(fd); masters.discard(fd)
def spawn(mode,terminal=False,args=None):
 master,slave=pty.openpty() if terminal else (None,None)
 def setup():
  os.setsid()
  if terminal:
   import fcntl,termios
   fcntl.ioctl(slave,termios.TIOCSCTTY,0)
 p=s.Popen([B]+(args or ['--','perl','workload.pl',mode]),cwd=P,env=E,stdin=slave if terminal else s.PIPE,stdout=slave if terminal else s.PIPE,stderr=slave if terminal else s.PIPE,preexec_fn=setup)
 if terminal: os.close(slave); masters.add(master)
 children.append(p); fd=master if terminal else p.stdout.fileno()
 data=line(fd)
 ready=data.split(b'READY:',1)[1].splitlines()[0].strip(); return p,fd,ready
name=None
try:
 run(['start']); marker=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project'); owner=json.loads(marker.read_text()); name=owner['name']; generation=owner['generation']; emit('kept box started; image='+image)
 def box(cmd): return run(['box','exec',name,'--','sh','-c',cmd]).stdout.decode().strip()
 def alive(pid): return box(f'test -r /proc/{pid}/stat && cat /proc/{pid}/stat || true').split(')')[-1].strip()[:1] not in ('','Z','X')
 def gone(pid):
  end=time.monotonic()+7
  while alive(pid):
   if time.monotonic()>end: raise RuntimeError('still alive '+str(pid))
 def peer():
  b.stdin.write(b'ping\n'); b.stdin.flush(); assert b'B-OK:ping' in line(b.stdout.fileno(),b'B-OK'); assert json.loads(marker.read_text())['generation']==generation
 b,_,bp=spawn('peer')
 for sig,code in [('INT',130),('TERM',143),('HUP',129),('KILL',-9)]:
  a,_,ready=spawn('plain'); pid=int(ready); t=time.monotonic(); a.send_signal(getattr(signal,'SIG'+sig)); assert a.wait(timeout=10)==code; gone(pid); peer(); emit(f'nonTTY SIG{sig}: client={code}, command gone, B=OK, elapsed={time.monotonic()-t:.2f}s')
 a,_,ready=spawn('ignore'); pid=int(ready); t=time.monotonic(); a.send_signal(signal.SIGTERM); assert a.wait(timeout=10)==143; assert alive(pid); gone(pid); elapsed=time.monotonic()-t; assert elapsed>=1.9; peer(); emit(f'ignore TERM: gone after {elapsed:.2f}s; B=OK')
 a,fd,ready=spawn('plain',True); pid=int(ready); closepty(fd); assert a.wait(timeout=10)==129; gone(pid); peer(); emit('terminal closure: client=129, command gone, B=OK')
 a,fd,_=spawn('',True,['--','sh','-i','-c','printf "READY:0\\n"; exec sh -i']); os.write(fd,b"printf 'CHILD-''READY\\n'; sleep 600\n"); line(fd,b'CHILD-READY'); os.write(fd,b'\x03'); os.write(fd,b"printf 'CTRL-''C-OK\\n'\n"); observed=line(fd,b'CTRL-C-OK'); os.write(fd,b'exec /bin/true\n'); code=wait_terminal(a,fd); assert code==0,(code,observed); closepty(fd); peer(); emit('TTY Ctrl-C: shell survived and ran CTRL-C-OK')
 a,_,ready=spawn('background'); parent,bg=map(int,ready.split(b':')); assert a.wait(timeout=10)==0; assert alive(bg); box('touch /tmp/write-background'); end=time.monotonic()+5
 while box('cat /tmp/background-wrote 2>/dev/null || true')!='OK':
  if time.monotonic()>end: raise RuntimeError('background writer failed')
 assert alive(bg); box(f'kill {bg}'); peer(); emit('normal completion: background child survives and writes after client exit; B=OK')
 a,_,ready=spawn('eof'); pid=int(ready); a.stdin.close(); line(a.stdout.fileno(),b'STDIN-EOF'); assert alive(pid) and a.poll() is None; a.send_signal(signal.SIGTERM); assert a.wait(timeout=10)==143; gone(pid); emit('plain stdin EOF: command still running; later departure ends it')
 for code in [0,23]:
  a,fd,_=spawn('',False,['--','sh','-c',f'echo READY:0; sleep 30 & exit {code}']); box('true'); a.send_signal(signal.SIGTERM); observed=a.wait(timeout=10); assert observed==code,(code,observed); emit(f'observed completion racing signal: status={observed}')
 a,_,ready=spawn('',False,['box','exec',name,'--','perl',str(P/'workload.pl'),'plain']); pid=int(ready); a.send_signal(signal.SIGTERM); a.wait(timeout=10); assert alive(pid); box(f'kill {pid}'); peer(); emit('public box exec departure: guest command survives')
 a,_,ready=spawn('escape'); parent,escaped=map(int,ready.split(b':')); a.send_signal(signal.SIGKILL); a.wait(); gone(parent); assert alive(escaped); peer(); emit('setsid: escaped child survives departure; B=OK'); run(['stop']); assert b.wait(timeout=10)==125; emit('owner teardown: incomplete client=125'); emit('stop: box absent='+str(run(['box','exec',name,'--','true'],False).returncode==3)); name=None
 emit('ALL CASES PASSED')
except BaseException as error:
 emit('CASE FAILED '+repr(error)); raise
finally:
 for fd in list(masters): closepty(fd)
 for p in children:
  if p.poll() is None: p.kill(); p.wait()
  if p.stderr is not None:
   os.set_blocking(p.stderr.fileno(),False)
   err=p.stderr.read()
   if err: emit("child stderr: "+err.decode())
 try:
  run(['stop'])
  for suffix in ['home','project']: run(['disk','rm','pi-'+project+'-'+suffix])
  emit('cleanup: own box stopped and two named fixture disks removed')
 except Exception as error: emit('CLEANUP FAILED '+str(error))
