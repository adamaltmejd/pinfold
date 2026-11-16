#!/usr/bin/env python3
"""Real guest transport diagnostic fault/control. Usage: diagnostics.py BIN IMAGE."""
import hashlib,json,os,pathlib,select,shutil,socket,struct,subprocess as sp,sys,tempfile,threading
BAD=b'fixture\x1b[31mBAD\x1b[0m\n'; SAFE=b'fixture\\u{1b}[31mBAD\\u{1b}[0m\\n'
def exact(stream,n):
 data=b''
 while len(data)<n:
  part=stream.read(n-len(data)) if hasattr(stream,'read') else stream.recv(n-len(data))
  if not part:raise EOFError
  data+=part
 return data
def send(stream,data):
 if hasattr(stream,'sendall'):stream.sendall(data)
 else:
  view=memoryview(data)
  while view:view=view[stream.write(view):]
  stream.flush()
def frame(stream):
 head=exact(stream,5);return head[:1],exact(stream,struct.unpack('>I',head[1:])[0])
def packet(kind,payload):return kind+struct.pack('>I',len(payload))+payload
def copy(source,dest):
 try:
  while True:
   data=os.read(source.fileno(),16384) if hasattr(source,'read') else source.recv(16384)
   if not data:break
   send(dest,data)
 except (EOFError,OSError):pass
 finally:
  if hasattr(dest,'shutdown'):
   try:dest.shutdown(socket.SHUT_WR)
   except OSError:pass
  else:
   try:dest.close()
   except OSError:pass
# This wrapper still executes the actual native runtime and guest command.
if len(sys.argv)>1 and sys.argv[1]=='--native':
 real,armed=sys.argv[2:4];args=sys.argv[4:]
 if args[-2:]!=['init','session-exec']:os.execv(real,[real]+args)
 p=sp.Popen([real]+args,stdin=sp.PIPE,stdout=sp.PIPE,stderr=None,bufsize=0)
 threading.Thread(target=copy,args=(sys.stdin.buffer,p.stdin),daemon=True).start()
 try:
  while True:
   kind,payload=frame(p.stdout)
   if kind==b'x' and pathlib.Path(armed).exists():kind,payload=b'F',BAD
   send(sys.stdout.buffer,packet(kind,payload))
 except (EOFError,OSError):pass
 sys.exit(p.wait())
B=str(pathlib.Path(sys.argv[1]).resolve());IMAGE=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfd.',dir='/tmp')).resolve();P=W/'p';P.mkdir();ARM=W/'armed'
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true')
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();IDENT='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
def emit(x):print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user' if sys.platform=='linux' else '/Users/user'),flush=True)
def run(args,check=True):
 r=sp.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=60)
 if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
if sys.platform=='darwin':
 (W/'bin').mkdir();real=shutil.which('container')
 wrapper='#!/bin/sh\nexec '+str(pathlib.Path(sys.executable))+' '+str(pathlib.Path(__file__).resolve())+' --native '+real+' '+str(ARM)+' "$@"\n'
 (W/'bin/container').write_text(wrapper);(W/'bin/container').chmod(0o755);E['PATH']=str(W/'bin')+os.pathsep+os.environ['PATH']
sp.run(['git','init','-q',str(P)],check=True);sp.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);conf=next((W/'c/pinfold/projects').glob('*/pinfold.toml'));conf.write_text(conf.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
listener=None;original=None;real_socket=W/'real.sock';stopping=threading.Event();proxy_errors=[]
def proxy(client):
 with client,socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as upstream:
  try:
   upstream.connect(str(real_socket));line=b''
   while not line.endswith(b'\n'):line+=exact(client,1)
   send(upstream,line);answer=b''
   while not answer.endswith(b'\n'):answer+=exact(upstream,1)
   send(client,answer)
   threading.Thread(target=copy,args=(client,upstream),daemon=True).start()
   while True:
    kind,payload=frame(upstream)
    if kind in [b'x',b'S'] and ARM.exists():kind,payload=b'F',BAD
    send(client,packet(kind,payload))
  except (EOFError,OSError):pass
  except BaseException as error:proxy_errors.append(repr(error))
def accept():
 while not stopping.is_set():
  try:client,_=listener.accept()
  except OSError:return
  threading.Thread(target=proxy,args=(client,),daemon=True).start()
try:
 run(['start']);marker=pathlib.Path('/tmp').resolve()/f'pinfold-{os.getuid()}'/(hashlib.sha256(b'project-owner:'+os.fsencode(P)).hexdigest()+'.project');owner=json.loads(marker.read_text());name=owner['name']
 if sys.platform=='linux':
  original=pathlib.Path('/tmp')/f'pinfold-{os.getuid()}'/('r-'+owner['generation'][:32])/'v/v.sock'
  original.rename(real_socket);listener=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);listener.bind(str(original));os.chmod(original,0o600);listener.listen();threading.Thread(target=accept,daemon=True).start()
 prefix=['box','exec',name,'--'] if sys.platform=='linux' else ['--']
 r=run(prefix+['true']);assert r.returncode==0;emit('real execution control: true status=0')
 r=run(prefix+['sh','-c','printf "\\033[31mDATA\\033[0m"']);assert r.stdout==b'\x1b[31mDATA\x1b[0m';emit('command stdout control: literal ESC bytes preserved')
 ARM.touch();r=run(prefix+['true'],False);expected=1 if sys.platform=='linux' else 125
 assert r.returncode==expected and SAFE in r.stderr and b'\x1b' not in r.stderr,(r.returncode,r.stderr)
 emit(('public Linux box exec' if sys.platform=='linux' else 'Apple project exec')+f': FAILED diagnostic escaped; status={r.returncode}; stderr={r.stderr!r}')
 ARM.unlink()
 if sys.platform=='linux':
  r=run(['box','stat',name]);assert json.loads(r.stdout);emit('public Linux stat control: real guest reply accepted')
  ARM.touch();r=run(['box','stat',name],False);assert r.returncode==1 and SAFE in r.stderr and b'\x1b' not in r.stderr;emit(f'public Linux stat: FAILED diagnostic escaped; status=1; stderr={r.stderr!r}');ARM.unlink()
 assert not proxy_errors,proxy_errors
 emit('DIAGNOSTIC CASES PASSED')
finally:
 ARM.unlink(missing_ok=True)
 if listener is not None:
  stopping.set();listener.close();original.unlink(missing_ok=True);real_socket.rename(original)
 run(['stop'])
 for suffix in ['home','project']:run(['disk','rm','pi-'+IDENT+'-'+suffix])
 emit('cleanup: original transport restored; own box and named disks removed')
