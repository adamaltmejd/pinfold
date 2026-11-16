#!/usr/bin/env python3
"""Real project-owner cases: cases.py BIN IMAGE [race|matrix]."""
import hashlib,http.server,json,os,pathlib,select,signal,socket,struct,subprocess as sp,sys,tempfile,threading
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2];MODE=sys.argv[3] if len(sys.argv)>3 else 'race'
W=pathlib.Path(tempfile.mkdtemp(prefix='pfo.',dir='/tmp')).resolve();P=W/'p';P.mkdir();HOST=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch()
def emit(s): print(str(s).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(a,code=0,data=b'',env=None,cwd=P):
 use=env or E;stamp=pathlib.Path(use['XDG_STATE_HOME'])/'pinfold/maintenance';stamp.parent.mkdir(parents=True,exist_ok=True);stamp.touch()
 r=sp.run([B]+a,cwd=cwd,env=use,input=data,capture_output=True,timeout=100)
 emit(f'{a!r} exit={r.returncode} out={r.stdout[:300]!r} err={r.stderr[:400]!r}')
 assert r.returncode==code,(a,r.returncode,r.stderr)
 return r
sp.run(['git','init','-q',str(P)],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);ID='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12];C=W/'c/pinfold/projects'/ID/'pinfold.toml';C.write_text(C.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n');BASE=C.read_text();server=None;receipts=[]
if MODE=='race':
 class Fixture(http.server.BaseHTTPRequestHandler):
  def do_GET(self):receipts.append(True);self.send_response(200);self.end_headers();self.wfile.write(b'RECEIPT')
  def log_message(self,*args):pass
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Fixture);threading.Thread(target=server.serve_forever,daemon=True).start();C.write_text(BASE+f'routes={{receipt="127.0.0.1:{server.server_port}"}}\n')
def proc(a,env=None,cwd=P):return sp.Popen([B]+a,cwd=cwd,env=env or E,stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE)
def line(p):
 assert select.select([p.stdout],[],[],100)[0],f'no guest marker, poll={p.poll()}'
 got=p.stdout.readline();assert got,(p.poll(),p.stderr.read());return got
L=HOST/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project')
def loc():return json.loads(L.read_text())
def complete(p,code):
 out,err=p.communicate(timeout=100);emit(f'client exit={p.returncode} out={out[:200]!r} err={err[:300]!r}');assert p.returncode in (code if isinstance(code,tuple) else (code,));return out
try:
 if MODE=='race':
  # First commands synchronize outside pinfold, then retain through input EOF.
  gate=threading.Barrier(3);children=[]
  def first(n):
   gate.wait();p=proc(['--','sh','-c',f'echo CLIENT-{n}; read x; curl -fsS http://receipt/; echo DONE-{n}']);children.append((n,p))
  ts=[threading.Thread(target=first,args=(n,)) for n in range(2)]
  [t.start() for t in ts];gate.wait();[t.join() for t in ts]
  for n,p in children:assert line(p)==f'CLIENT-{n}\n'.encode()
  old=loc();emit(f'simultaneous-first one-generation={old["generation"]} clients=2')
  a,b=[p for _,p in children];a.kill();complete(a,-9)
  b.stdin.write(b'continue\n');b.stdin.flush();complete(b,0);assert receipts;emit('surviving client fixture request=RECEIPT after peer SIGKILL')
  run(['stop']);assert not L.exists();emit('first-client-kill survivor completed; last-departure retired')
  # Race release of the last connection with keep and another join.
  for verb in ('join','keep','stop'):
   p=proc(['--','sh','-c','echo LAST; read x']);assert line(p)==b'LAST\n';old=loc();bar=threading.Barrier(3);result=[]
   def leave():bar.wait();p.stdin.write(b'end\n');p.stdin.flush();result.append(('leave',complete(p,(0,125) if verb=='stop' else 0)))
   def admit():
    bar.wait();args=['--','sh','-c','echo RACE-MARKER'] if verb=='join' else ['start'] if verb=='keep' else ['stop'];r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=100);result.append((verb,r.returncode,r.stdout,r.stderr))
   t1=threading.Thread(target=leave);t2=threading.Thread(target=admit);t1.start();t2.start();bar.wait();t1.join();t2.join();emit(f'final-departure-{verb}: {result!r}');assert len(result)==2;assert all(x[1]==0 for x in result if x[0]!='leave')
   run(['stop'])
  run(['start']);old=loc();run(['start']);assert loc()==old
  p=proc(['--','sh','-c','echo ACTIVE; read x']);assert line(p)==b'ACTIVE\n';p.send_signal(signal.SIGTERM);p.wait(timeout=100);complete(p,143);assert loc()==old
  run(['--','sh','-c','echo KEPT-SURVIVES']);run(['stop']);emit('kept-without-clients and signal-departure passed')
 elif MODE=='compat':
  if sys.platform=='linux':
   def spec(n):return dict(name='probe-owner-cap-'+str(os.getpid()),image=IMAGE,cpus=2,memory='1G',disks=[dict(name=f'probe-owner-cap-{os.getpid()}-{i}',guest=f'/d{i}',size='1G') for i in range(n)])
   r=run(['box','up'],1,data=json.dumps(spec(15)).encode()+b'\n');assert json.loads(r.stdout)['event']=='refused'
   p=proc(['box','up']);p.stdin.write(json.dumps(spec(14)).encode()+b'\n');p.stdin.flush();ready=json.loads(line(p));assert ready['event']=='ready' and not L.exists()
   assert run(['box','exec',ready['box'],'--','printf','FOURTEEN']).stdout==b'FOURTEEN';run(['box','stat',ready['box']]);assert not L.exists();p.stdin.close();p.stdin=None;complete(p,0)
   emit('Linux attachment 15 refused; 14 ran/exec/stat; public stdin EOF down; raw exec created no project lease')
  plain=W/'plain';plain.mkdir();(plain/'host-only').write_text('HOST')
  if sys.platform=='linux':assert b'not-git' in run(['--','true'],125,cwd=plain).stderr
  else:
   run(['config','edit'],cwd=plain);pc=W/'c/pinfold/projects'/('plain-'+hashlib.sha256(os.fsencode(plain)).hexdigest()[:12])/'pinfold.toml';pc.write_text(pc.read_text()+f'image={json.dumps(IMAGE)}\n');run(['--','sh','-c','test ! -e host-only; printf PLAIN > kept'],cwd=plain);assert run(['--','cat','kept'],cwd=plain).stdout==b'PLAIN'
  sp.run(['git','-C',str(P),'worktree','add','-qb','probe-linked',str(W/'linked')],check=True,capture_output=True);assert b'worktree' in run(['--','true'],125,cwd=W/'linked').stderr
  inside=P/'inside';(W/'state-link').symlink_to(inside,target_is_directory=True);e=dict(E,XDG_CACHE_HOME=str(W/'state-link/new/cache'));r=sp.run([B,'--','true'],cwd=P,env=e,input=b'',capture_output=True,timeout=90);assert r.returncode==125 and b'host-path-in-project' in r.stderr and not inside.exists()
  run(['--','true']);emit('protected dangling-ancestor path and worktree refusals left target absent; Git/external-root control succeeded; platform plain-folder contract passed')

 else:
  run(['start']);old=loc();
  mutations=[('resources.cpus',{'PINFOLD_CPUS':'1'}),('resources.memory',{'PINFOLD_MEMORY':'512M'}),('env.names',{'PINFOLD_ENV_NEW':'value'}),('roots.state',{'XDG_STATE_HOME':str(W/'other-state')}),('roots.config',{'XDG_CONFIG_HOME':str(W/'missing-config')}),('roots.home',{'HOME':str(W/'home')})]
  for category,changes in mutations:
   e=dict(E);e.update(changes);r=run(['--','sh','-c','echo MUST-NOT-RUN'],125,env=e);assert b'config-changed' in r.stderr and not r.stdout;assert loc()==old;run(['--','true'])
  for name,text in [('allow','allow=["example.com"]\n'),('route','routes={fixture="127.0.0.1:1"}\n'),('profile','profile="documents"\n')]:
   C.write_text(BASE.replace('allow=[]\n',text) if name=='allow' else BASE+text);r=run(['start'],125);assert b'config-changed' in r.stderr;C.write_text(BASE);run(['--','true']);assert loc()==old
  for code in (0,23,125,126,127):run(['--','sh','-c',f'exit {code}'],code)
  run(['--','definitely-not-present'],125)
  run(['--','sh','-c','cat; printf \"ERR\\000\" >&2'],data=b'A\x00B')
  # Readiness/Git and command metadata excluded from policy equality.
  (P/'sub').mkdir();run(['--','mkdir','sub']);run(['--','pwd'],cwd=P/'sub');assert loc()==old
  assert run(['--','printf','<%s>\n','','space words','a;$(echo bad)','--help']).stdout==b'<>\n<space words>\n<a;$(echo bad)>\n<--help>\n'
  run(['--', 'sh', '-c', 'mkdir real; chmod 111 real; ln -s real internal; ln -s /tmp external; printf \'%s\\n\' \'#!/bin/sh\' \'printf "%s" "$HERDR_AGENT"\' > probe-agent; chmod 755 probe-agent'])
  for d in ('internal','external','missing'):
   (P/d).mkdir();r=run(['--','sh','-c','printf MUST-NOT-RUN'],125,cwd=P/d);assert not r.stdout
  (P/'real').mkdir();assert run(['--','pwd'],cwd=P/'real').stdout.strip()==os.fsencode(P/'real')
  assert run(['--','./probe-agent']).stdout==b'./probe-agent';assert loc()==old
  emit('literal argv/help, guest symlink/missing cwd refusals, execute-only cwd and exact Herdr first-word label passed in same generation')
  # Bad frames create no lease or keep changes; valid command still joins.
  sock=HOST/('r-'+old['generation'][:32])/'c.sock'
  for payload in (b'{',json.dumps({'version':2,'request':{'verb':'leave'}}).encode(),json.dumps({'version':1,'request':{'verb':'leave','unknown':1}}).encode()):
   s=socket.socket(socket.AF_UNIX);s.connect(str(sock));s.sendall(struct.pack('!I',len(payload))+payload);n=struct.unpack('!I',s.recv(4))[0];answer=json.loads(s.recv(n));assert answer['reply']['reason']=='session-protocol';s.close()
  for length,body in ((17*1024*1024,b''),(32,b'{')):
   s=socket.socket(socket.AF_UNIX);s.settimeout(3);s.connect(str(sock));s.sendall(struct.pack('!I',length)+body);s.shutdown(socket.SHUT_WR);n=struct.unpack('!I',s.recv(4))[0];answer=json.loads(s.recv(n));assert answer['reply']['reason']=='session-protocol';s.close()
  slow=socket.socket(socket.AF_UNIX);slow.connect(str(sock));slow.sendall(struct.pack('!I',100)+b'{');run(['--','true']);slow.settimeout(3);n=struct.unpack('!I',slow.recv(4))[0];assert json.loads(slow.recv(n))['reply']['reason']=='session-protocol';slow.close()
  s=socket.socket(socket.AF_UNIX);s.settimeout(3);s.connect(str(sock));payload=json.dumps({'version':1,'request':{'verb':'probe','root':str(P),'generation':'0'*64,'roots':dict(config='',state='',cache='',home='',codex_home=None)}}).encode();s.sendall(struct.pack('!I',len(payload))+payload);n=struct.unpack('!I',s.recv(4))[0];assert json.loads(s.recv(n))['reply']['reason']=='session-protocol';s.close()
  emit('malformed/unknown/version/oversize/truncated/stalled/wrong-generation frames refused without keep/policy change');run(['--','true']);assert loc()==old
  run(['stop']);terminal=json.loads((W/'s/pinfold/runs'/old['generation']/'terminal.json').read_text());emit(f'stop-terminal={terminal}')
finally:
 try:run(['stop'])
 except Exception as error:emit(f'cleanup-stop-failed={error}')
 try:run(['disk','rm','--all'])
 except Exception as error:emit(f'cleanup-disk-failed={error}')
 if server:server.shutdown();server.server_close()
 emit(f'fixture={W}')
