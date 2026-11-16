import hashlib,http.server,json,os,pathlib,pty,select,signal,subprocess as s,sys,tempfile,threading,time
B=str(pathlib.Path(sys.argv[1]).resolve());I=sys.argv[2];D=pathlib.Path(__file__).resolve().parent
W=pathlib.Path(tempfile.mkdtemp(prefix='pfp.',dir='/tmp')).resolve();P=W/'p';P.mkdir()
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();hits={};cv=threading.Condition()
def emit(x):print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(a):
 r=s.run([B]+a,cwd=P,env=E,input=b'',capture_output=True,timeout=90)
 if r.returncode:raise RuntimeError((a,r.returncode,r.stderr.decode()))
 return r.stdout.decode()
class Fixture(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  with cv:hits[self.path]=hits.get(self.path,0)+1;cv.notify_all()
  self.send_response(200);self.end_headers();self.wfile.write(b'OK')
 def log_message(self,*a):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Fixture);threading.Thread(target=server.serve_forever,daemon=True).start()
s.run(['git','init','-q',str(P)],check=True);(P/'workload.pl').write_bytes((D/'workload.pl').read_bytes());s.run(['git','-C',str(P),'add','.'],check=True);s.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','-qm','fixture'],check=True)
run(['config','edit']);C=W/'c/pinfold/projects'/('p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12])/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(I)}\nallow=[]\nroutes={{receipt="127.0.0.1:{server.server_port}"}}\n')
def ready(fd):
 buf=b'';end=time.monotonic()+90
 while b'READY' not in buf:
  if not select.select([fd],[],[],max(0,end-time.monotonic()))[0]:raise RuntimeError(('readiness',buf))
  buf+=os.read(fd,4096)
 return buf
try:
 emit('start '+run(['start']).strip());emit('root '+str(W));L=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project');generation=json.loads(L.read_text())['generation'];emit('generation '+generation)
 probe='uname -r; cat /proc/self/cgroup; cat /proc/sys/user/max_user_namespaces; grep cgroup /proc/mounts; for p in 1 $PPID $$; do echo PROCESS=$p; cat /proc/$p/comm; grep -E "^(PPid|Uid|Gid|Cap|NoNewPrivs):|^Cap" /proc/$p/status; done; ls -ld /sys/fs/cgroup; cat /sys/fs/cgroup/cgroup.controllers; test -e /sys/fs/cgroup/cgroup.kill && echo cgroup.kill=present; ls -l /sys/fs/cgroup/cgroup.procs; cat /sys/fs/cgroup/work/pids.max /sys/fs/cgroup/work/cgroup.kill 2>/dev/null || :; unshare -U true'
 emit('GUEST\n'+run(['--','sh','-c',probe+'; true']))
 for tty in (False,True):
  for departure in ('INT','TERM','KILL','close'):
   tag=('tty' if tty else 'pipe')+departure;master,slave=pty.openpty() if tty or departure=='close' else (None,None)
   def child_setup():
    os.setsid()
    if slave is not None:
     import fcntl,termios
     fcntl.ioctl(slave,termios.TIOCSCTTY,0)
   a=s.Popen([B,'--','perl','workload.pl',tag],cwd=P,env=E,stdin=slave if slave is not None else s.PIPE,stdout=slave if tty else s.PIPE,stderr=slave if tty else s.PIPE,start_new_session=False,preexec_fn=child_setup)
   if slave is not None:os.close(slave)
   ready(master if tty else a.stdout.fileno())
   b=s.Popen([B,'--','sh','-c','echo READY; read x; curl -fsS http://receipt/control'],cwd=P,env=E,stdin=s.PIPE,stdout=s.PIPE,stderr=s.PIPE);ready(b.stdout.fileno())
   roles=['ignore','double','setsid','forker']
   with cv:
    assert cv.wait_for(lambda:all('/'+tag+'/'+r in hits for r in roles),10)
    before={r:hits['/'+tag+'/'+r] for r in roles}
   if departure=='close':
    if tty:os.close(master);master=None
    else:os.close(master);master=None
   else:a.send_signal(getattr(signal,'SIG'+departure))
   try:a.wait(timeout=15)
   except s.TimeoutExpired:raise RuntimeError(tag+' client did not exit in 15s')
   with cv:before={r:hits.get('/'+tag+'/'+r,0) for r in roles}
   with cv:cv.wait_for(lambda:all(hits.get('/'+tag+'/'+r,0)>before[r] for r in roles),4)
   status=run(['--','sh','-c',f'while read role p; do if test -e /proc/$p/status; then printf "$role:$p:"; grep "^State:" /proc/$p/status; fi; done < /tmp/{tag}.pids'])
   b.stdin.write(b'go\n');b.stdin.flush();out,err=b.communicate(timeout=20)
   assert json.loads(L.read_text())['generation']==generation
   emit(f'{tag} same_generation=True client={a.poll()} live={status.strip()!r} fixture_after={[r for r in roles if hits.get("/"+tag+"/"+r,0)>before[r]]} control={b.returncode}:{out.decode().strip()}')
   run(['--','sh','-c',f'while read role p; do kill -KILL "$p" 2>/dev/null || :; done < /tmp/{tag}.pids'])
   if a.poll() is None:a.kill();a.wait()
   if master is not None:os.close(master)
finally:
 try:emit('stop '+run(['stop']).strip());emit('disks '+run(['disk','rm','--all']).strip())
 finally:server.shutdown()
