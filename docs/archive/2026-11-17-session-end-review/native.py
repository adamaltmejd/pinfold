#!/usr/bin/env python3
"""Real Apple native-client exit with a descendant holding stdout."""
import hashlib,json,os,pathlib,select,shutil,signal,subprocess as s,sys,tempfile,time
B=str(pathlib.Path(sys.argv[1]).resolve()); I=sys.argv[2]
W=pathlib.Path(tempfile.mkdtemp(prefix='pfn.',dir='/tmp')).resolve();P=W/'p';P.mkdir();(W/'bin').mkdir()
real=shutil.which('container');marker=W/'native-pids'
# The real native client runs unchanged. The fixture only retains its output fd.
(W/'bin/container').write_text('#!/bin/sh\ncase " $* " in *" init session-exec"*) sleep 60 & echo "$$ $!" > '+str(marker)+' ;; esac\nexec '+real+' "$@"\n');(W/'bin/container').chmod(0o755)
E={k:v for k,v in os.environ.items() if not k.startswith('PINFOLD_')};E.update(XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G',EDITOR='true',VISUAL='true',PATH=str(W/'bin')+os.pathsep+os.environ['PATH'])
(W/'s/pinfold').mkdir(parents=True);(W/'s/pinfold/maintenance').touch();ident='p-'+hashlib.sha256(os.fsencode(P)).hexdigest()[:12]
def emit(x):print(str(x).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/Users/user'),flush=True)
def run(args,check=True):
 r=s.run([B]+args,cwd=P,env=E,input=b'',capture_output=True,timeout=90)
 if check and r.returncode:raise RuntimeError((args,r.returncode,r.stderr.decode()))
 return r
s.run(['git','init','-q',str(P)],check=True);s.run(['git','-C',str(P),'-c','user.name=user','-c','user.email=user@invalid','commit','--allow-empty','-qm','fixture'],check=True)
run(['config','edit']);C=next((W/'c/pinfold/projects').glob('*/pinfold.toml'));C.write_text(C.read_text()+f'image={json.dumps(I)}\nallow=[]\n');children=[]
try:
 run(['start'])
 for code in [0,23]:
  t=time.monotonic();r=run(['--','sh','-c','exit '+str(code)],False);assert r.returncode==code,(r.returncode,r.stderr);emit(f'native stdout held by descendant: observed status={code}, elapsed={time.monotonic()-t:.2f}s')
 p=s.Popen([B,'--','sh','-c','echo NATIVE_READY; read x'],cwd=P,env=E,stdin=s.PIPE,stdout=s.PIPE,stderr=s.PIPE);children.append(p)
 assert select.select([p.stdout],[],[],15)[0];assert p.stdout.readline()==b'NATIVE_READY\n'
 native,desc=map(int,marker.read_text().split());os.kill(desc,0);t=time.monotonic();os.kill(native,signal.SIGKILL);assert p.wait(timeout=4)==125
 emit(f'native SIGKILL with descendant holding stdout: client=125, elapsed={time.monotonic()-t:.2f}s')
 emit('NATIVE EXIT CASES PASSED')
finally:
 for p in children:
  if p.poll() is None:p.kill();p.wait(timeout=10)
 run(['stop'])
 for suffix in ['home','project']:run(['disk','rm','pi-'+ident+'-'+suffix])
 emit('cleanup: own box and named disks removed')
