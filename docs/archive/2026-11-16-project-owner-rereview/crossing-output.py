#!/usr/bin/env python3
"""Real completed transfer with blocked caller output; BIN IMAGE."""
import fcntl,json,os,pathlib,select,signal,subprocess as sp,sys,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/crossing.py'
exec(compile(old.read_text().split('for p in (P, Q, INTO):')[0],str(old),'exec'),globals())
P.mkdir();git(P,'init','-q','-b','rereview');(P/'tracked').write_text('before')
git(P,'add','tracked');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','before')
run(['config','edit']);c=W/'c/pinfold/projects'/ident(P)/'pinfold.toml';c.write_text(c.read_text()+f'image={json.dumps(IMAGE)}\nallow=[]\n')
child=observer=None
try:
 run(['start']);observed=loc(P)
 (P/'tracked').write_text('after');git(P,'add','tracked');git(P,'-c','user.name=user','-c','user.email=user@pinfold.invalid','commit','-qm','after')
 expected=git(P,'rev-parse','HEAD')
 observer=sp.Popen([B,'--','sh','-c','until [ "$(git rev-parse refs/remotes/host/rereview 2>/dev/null)" = "$1" ]; do :; done; printf REF-COMMITTED','sh',expected.decode()],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=sp.PIPE)
 read,write=os.pipe();flags=fcntl.fcntl(write,fcntl.F_GETFL);fcntl.fcntl(write,fcntl.F_SETFL,flags|os.O_NONBLOCK)
 try:
  while True:os.write(write,b'F'*4096)
 except BlockingIOError:pass
 fcntl.fcntl(write,fcntl.F_SETFL,flags);flags=fcntl.fcntl(write,fcntl.F_GETFL)
 try:
  child=sp.Popen([B,'push'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=write,stderr=sp.PIPE)
  assert select.select([observer.stdout],[],[],90)[0] and observer.stdout.read(13)==b'REF-COMMITTED'
  assert observer.wait(timeout=10)==0 and child.poll() is None
  began=time.monotonic();child.send_signal(signal.SIGTERM);assert child.wait(timeout=6)==143
  assert fcntl.fcntl(write,fcntl.F_GETFL)==flags;elapsed=time.monotonic()-began;same(P,observed)
  emit(f'blocked-crossing-output: guest ref independently committed; SIGTERM=143 seconds={elapsed:.2f} caller-flags=unchanged owner=same-generation; parent did not drain stdout')
 finally:os.close(write);os.close(read)
 control=json.loads(run(['push']).stdout);assert control['event']=='pushed' and control['refs']['refs/remotes/host/rereview'].encode()==expected
 assert run(['--','printf','CROSSING-CONTROL']).stdout==b'CROSSING-CONTROL';same(P,observed)
 emit('crossing-output control: JSON=pushed independent-ref=host-HEAD same-owner CMD=0/CROSSING-CONTROL')
finally:
 for process in (child,observer):
  if process is not None and process.poll() is None:process.terminate();process.communicate(timeout=90)
 run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 emit('cleanup own boxes/disks=True')
