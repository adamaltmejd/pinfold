#!/usr/bin/env python3
"""Real completed-status/failed-output cases: status.py BIN IMAGE."""
import fcntl,os,pathlib,subprocess as sp,sys,time
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-review/teardown.py'
source=old.read_text().split('try:\n if len(sys.argv)>3')[0].replace('expected=code or 125','expected=code')
exec(compile(source,str(old),'exec'),globals())
try:
 for code in (0,23):cleanup_failure(code)
 run(['start'])
 for code in (0,23):
  target='/dev/full' if sys.platform=='linux' else '/dev/null'
  flags=os.O_WRONLY if sys.platform=='linux' else os.O_RDONLY
  fd=os.open(target,flags)
  try:
   result=sp.run([B,'--','sh','-c',f'printf RELAY; exit {code}'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=fd,stderr=sp.PIPE,timeout=90)
  finally:os.close(fd)
  assert result.returncode==code,(result.returncode,result.stderr)
  assert b'relay' in result.stderr or b'transport' in result.stderr,result.stderr
  emit(f'failed-output observed={code} client={result.returncode} diagnostic=present target='+target)
 assert run(['--','printf','WRITABLE-CONTROL']).stdout==b'WRITABLE-CONTROL'
 emit('writable-output control=0/WRITABLE-CONTROL')
 read,write=os.pipe();flags=fcntl.fcntl(write,fcntl.F_GETFL)
 fcntl.fcntl(write,fcntl.F_SETFL,flags|os.O_NONBLOCK)
 try:
  while True:os.write(write,b'F'*4096)
 except BlockingIOError:pass
 fcntl.fcntl(write,fcntl.F_SETFL,flags)
 flags=fcntl.fcntl(write,fcntl.F_GETFL)
 fd=os.open('/dev/full',os.O_WRONLY) if sys.platform=='linux' else os.open('/dev/null',os.O_RDONLY)
 try:
  began=time.monotonic();child=sp.Popen([B,'--','sh','-c','printf RELAY; exit 0'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=fd,stderr=write)
  assert child.wait(timeout=6)==0
  elapsed=time.monotonic()-began;assert fcntl.fcntl(write,fcntl.F_GETFL)==flags and elapsed<6
  emit(f'full-stderr relay failure: completed=0 client=0 seconds={elapsed:.2f} caller-flags=unchanged; parent never drained stderr')
  if sys.platform=='linux':
   # The trial VM is exclusive; never inject a global unreadable claim on the shared Mac.
   name='probe-rereview-diagnostic-'+str(os.getpid());fake=H/(hashlib.sha256(('box:'+name).encode()).hexdigest()+'.owner')
   fd_claim=os.open(fake,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.write(fd_claim,b'malformed-fixture');os.close(fd_claim)
   try:
    began=time.monotonic();r=sp.run([B,'--','printf','DISCOVERY-CONTROL'],cwd=P,env=E,stdin=sp.DEVNULL,stdout=sp.PIPE,stderr=write,timeout=8)
    assert r.returncode==0 and r.stdout==b'DISCOVERY-CONTROL';assert fcntl.fcntl(write,fcntl.F_GETFL)==flags
    emit(f'full-stderr discovery warning: malformed own fixture claim; joined CMD=0/DISCOVERY-CONTROL seconds={time.monotonic()-began:.2f} caller-flags=unchanged')
   finally:
    assert fake.is_file() and fake.read_bytes()==b'malformed-fixture';fake.unlink()
   assert run(['--','printf','DISCOVERY-RESTORED']).stdout==b'DISCOVERY-RESTORED'
   emit('discovery control: own malformed claim removed; same-owner CMD=0/DISCOVERY-RESTORED')

 finally:os.close(fd);os.close(write);os.close(read)

finally:
 run(['stop'])
 for disk in ('pi-'+ID+'-home','pi-'+ID+'-project'):run(['disk','rm',disk])
