#!/usr/bin/env python3
"""Linux real filesystem failure after terminal write, followed by owner retry: BIN IMAGE."""
import shlex,threading,time
import json,os,pathlib,select,subprocess as sp,sys
assert sys.platform=='linux'
base=pathlib.Path(__file__).resolve().parents[1]/'2026-11-16-project-owner-rereview-2/audit-leave.py'
exec(compile(base.read_text().split('def line(')[0],str(base),'exec'),globals())
R=pathlib.Path(__file__).resolve().parents[3]
def anchor(path,text):
 found=[i+1 for i,line in enumerate(path.read_text().splitlines()) if text in line];assert len(found)==1,(text,found)
 return str(path)+':'+str(found[0])
BOX=R/'crates/pinfold/src/core/box.rs';SESSION=R/'crates/pinfold/src/pi/session.rs'
finish=anchor(BOX,'let cleanup = self.ownership.finish(result.is_ok()).await;');retry=anchor(SESSION,'let down = match owned_box.as_mut()')
def case(fault):
 global receipt,release
 receipt,release=threading.Event(),threading.Event();mode='retry' if fault else 'control'
 fifo=W/(mode+'.stdin');os.mkfifo(fifo);input_fd=os.open(fifo,os.O_RDWR|os.O_NONBLOCK)
 out,err,status,snapshot,summary,script,debuglog=[W/(mode+'.'+suffix) for suffix in ('out','err','status','snapshot','summary','gdb','debug')]
 command='set -e; curl -fsS http://audit/control; curl -fsS http://audit/hold </dev/null >/dev/null 2>&1 & echo READY; read x; sync; exit 0'
 script.write_text(f'''set pagination off
set confirm off
set print thread-events off
set detach-on-fork on
set follow-fork-mode parent
handle SIGPIPE nostop noprint pass
python
import gdb,hashlib,json,os,pathlib,threading
starter=None
status_thread=None
writes=0
swapped=False
restored=False
claim=None
backup=None
marker=None
def status():
 pid,value=os.waitpid(starter,0);pathlib.Path({str(status)!r}).write_text(str(os.waitstatus_to_exitcode(value)))
class Spawn(gdb.Breakpoint):
 def stop(self):
  global starter
  starter=gdb.selected_inferior().pid;self.enabled=False;gdb.execute('set follow-fork-mode child');return False
class Main(gdb.Breakpoint):
 def stop(self):
  global status_thread
  if starter is not None and gdb.selected_inferior().pid!=starter:
   self.enabled=False;gdb.execute('set follow-fork-mode parent');status_thread=threading.Thread(target=status);status_thread.start()
  return False
class Written(gdb.Breakpoint):
 def stop(self):
  global writes
  writes+=1;return False
class BeforeFinish(gdb.Breakpoint):
 def stop(self):
  global swapped,claim,backup,marker
  self.enabled=False
  loc=json.loads(pathlib.Path({str(locator(P))!r}).read_text());terminal=pathlib.Path({str(W/'s/pinfold/runs')!r})/loc['generation']/'terminal.json'
  data=terminal.read_bytes();record=json.loads(data);assert record['reason']=='audit-log' and record['detail'] is None
  value=dict(generation=loc['generation'],bytes=data.hex(),inode=terminal.stat().st_ino,time=record['time'])
  pathlib.Path({str(snapshot)!r}).write_text(json.dumps(value))
  if {fault!r}:
   claim=pathlib.Path({str(H)!r})/(hashlib.sha256(('box:'+loc['name']).encode()).hexdigest()+'.owner');backup=pathlib.Path({str(W/(mode+'.owner.saved'))!r});claim.rename(backup);claim.mkdir(mode=0o700);marker=claim/'fixture';marker.write_bytes(b'OWN-FINISH-FAILURE');swapped=True
  return False
class Retry(gdb.Breakpoint):
 def stop(self):
  global restored,swapped
  if swapped and not restored:
   assert marker.read_bytes()==b'OWN-FINISH-FAILURE';marker.unlink();claim.rmdir();backup.rename(claim);restored=True;swapped=False
  return False
Spawn('pinfold::pi::session::spawn_owner');Main('pinfold::main');Written('pinfold::core::runs::write_terminal');BeforeFinish({finish!r});Retry({retry!r})
end
run -- sh -c {shlex.quote(command)} < {shlex.quote(str(fifo))} > {shlex.quote(str(out))} 2> {shlex.quote(str(err))}
python
if status_thread is not None:
 status_thread.join(90);assert not status_thread.is_alive(),'starter survived owner completion'
pathlib.Path({str(summary)!r}).write_text(json.dumps(dict(writes=writes,restored=restored)))
end
''')
 with debuglog.open('wb') as output:debug=sp.Popen(['gdb','-nx','-q','-batch','-x',str(script),'--args',B],cwd=P,env=E,stdin=sp.DEVNULL,stdout=output,stderr=sp.STDOUT)
 log=saved=None
 try:
  assert receipt.wait(90),'real pending route barrier missing';observed=loc(P);generation=observed['generation'];log=W/'s/pinfold/runs'/generation/'egress.jsonl';saved=log.with_suffix('.saved');log.rename(saved);log.mkdir(mode=0o700)
  os.write(input_fd,b'go\n');debug.communicate(timeout=150);assert debug.returncode==0,debuglog.read_bytes()[-1800:]
  assert status.read_text()=='0' and out.read_bytes()==b'AUDIT-CONTROL\nREADY\n'
  first=json.loads(snapshot.read_text());counts=json.loads(summary.read_text());terminal=W/'s/pinfold/runs'/generation/'terminal.json';current=terminal.read_bytes()
  assert counts==dict(writes=1,restored=fault),counts
  assert current.hex()==first['bytes'] and terminal.stat().st_ino==first['inode'] and json.loads(current)['time']==first['time']
  assert b'audit-log' in err.read_bytes() and generation.encode() in err.read_bytes()
  if fault:assert b'teardown-failed' in err.read_bytes()
  else:assert b'teardown-failed' not in err.read_bytes()
  assert not locator(P).exists()
  emit(f'terminal-{mode}: real pending route audit failure; '+('own record path became directory for first finish, then restored before owner retry; ' if fault else 'ordinary audit departure; ')+f'write calls=1 owner-retry={counts["restored"]}; timestamp/content/inode=unchanged; client=0 audit-log=reported; locator=absent')
 finally:
  release.set();os.close(input_fd)
  if debug.poll() is None:debug.terminate();debug.communicate(timeout=15)
  if log is not None and log.is_dir():log.rmdir()
  if saved is not None and saved.exists():saved.rename(log)
  # Only our exact fault marker may authorize restoring this fixture record.
  backup=W/(mode+'.owner.saved')
  if backup.exists():
   observed=json.loads(backup.read_text());claim=H/(hashlib.sha256(('box:'+observed['name']).encode()).hexdigest()+'.owner');marker=claim/'fixture'
   assert marker.read_bytes()==b'OWN-FINISH-FAILURE';marker.unlink();claim.rmdir();backup.rename(claim)
  run(['stop'])
 assert run(['--','printf','TERMINAL-CONTROL']).stdout==b'TERMINAL-CONTROL';run(['stop']);emit(f'terminal-{mode} restart=0/TERMINAL-CONTROL')
try:
 case(False);case(True)
finally:
 release.set();run(['stop'])
 for disk in ('pi-'+ident(P)+'-home','pi-'+ident(P)+'-project'):run(['disk','rm',disk])
 server.shutdown();server.server_close();server_thread.join();emit('cleanup exact own boxes/disks/record fixtures=True')
