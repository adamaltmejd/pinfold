#!/usr/bin/env python3
"""Two independent real project PTYs: terminals.py BIN IMAGE; manual evidence."""
import contextlib,io,json,os,pathlib,select,signal,sys
old=pathlib.Path(__file__).resolve().parents[1]/'2026-11-15-project-command-review/tty-review.py'
source=old.read_text().split('emit("phase="')[0];sys.argv=sys.argv[:3]+['fixed']
source=source.replace('            if select.select([master], [], [], 0.2)[0]:','''            if select.select([resize_fd], [], [], 0)[0]:
                assert os.read(resize_fd,1)==b'R'
                fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack("HHHH",*new_size,0,0))
            if select.select([master], [], [], 0.2)[0]:''')
needle='            if interrupt and b"CMD-READY\\n" in plain and not sent:'
source=source.replace(needle,'''            if b"TTY_READY\\n" in plain and not notified:
                os.write(ready_fd,b'R');notified=True
'''+needle).replace('    output, sent = b"", False','    output, sent = b"", False;notified=False')
assert source.count('os.write(ready_fd')==1 and source.count('select.select([resize_fd]')==1
exec(compile(source,str(old),'exec'),globals())
env['PINFOLD_ENV_HERDR_AGENT']='conflicting-fixture'
def run(args):
 r=sp.run([binary]+args,cwd=root,env=env,input=b'',capture_output=True,timeout=90)
 assert r.returncode==0,(args,r.returncode,r.stderr[:300]);return r
key=hashlib.sha256(b'project-owner:'+os.fsencode(root)).hexdigest();locator=Path('/tmp')/f'pinfold-{os.getuid()}'/(key+'.project')
children=[];channels=[]
program='$|=1; $SIG{WINCH}=sub {print "RESIZE="; system("stty","size"); exit 0}; print "FIRST=$ENV{TERM}:$ENV{COLORTERM}:$ENV{HERDR_AGENT}:"; system("stty","size"); print "TTY_READY\\n"; while(1){sysread(STDIN,$b,1)}'
try:
 run(['start']);generation=json.loads(locator.read_text())
 for index,(label,term,color,size,new_size) in enumerate([('/usr/bin/perl','xterm-256color','truecolor',(22,71),(41,101)),('perl','vt100','24bit',(33,92),(51,121))]):
  ready_r,ready_fd=os.pipe();resize_fd,resize_w=os.pipe();result_r,result_w=os.pipe();pid=os.fork()
  if pid==0:
   os.close(ready_r);os.close(resize_w);os.close(result_r);env.update(TERM=term,COLORTERM=color)
   try:
    with contextlib.redirect_stdout(io.StringIO()):code,equal,sane,observed=terminal(['--',label,'-e',program],size=size)
    result={'code':code,'restored':equal,'sane':sane,'observed':observed.decode('utf-8','backslashreplace')}
   except BaseException as error:result={'error':repr(error)}
   os.write(result_w,json.dumps(result).encode());os._exit(0)
  os.close(ready_fd);os.close(resize_fd);os.close(result_w);children.append(pid);channels.append((ready_r,resize_w,result_r,label,term,color,size,new_size))
 for ready_r,*rest in channels:
  assert select.select([ready_r],[],[],90)[0] and os.read(ready_r,1)==b'R','guest readiness absent'
 for index,(ready_r,resize_w,result_r,label,term,color,size,new_size) in enumerate(channels):
  os.write(resize_w,b'R');assert select.select([result_r],[],[],90)[0],'resize completion absent';result=json.loads(os.read(result_r,65536))
  assert result.get('code')==0 and result.get('restored') and result.get('sane'),result
  expected=f'FIRST={term}:{color}:{label}:{size[0]} {size[1]}\n';resized=f'RESIZE={new_size[0]} {new_size[1]}\n'
  assert expected in result['observed'] and resized in result['observed'],result
  assert result['observed'].count('RESIZE=')==1,result
  if index==0:assert not select.select([channels[1][2]],[],[],0)[0],'second terminal completed before its resize'
  waited,status=os.waitpid(children[index],0);assert os.waitstatus_to_exitcode(status)==0
  assert json.loads(locator.read_text())==generation,'PTY command changed kept generation'
  emit(f'independent-pty-{index+1}: '+json.dumps(result,sort_keys=True))
 emit('PASS simultaneous first dimensions, TERM/COLORTERM, exact HERDR_AGENT, independent resize and full terminal restoration; same kept generation')
finally:
 for pid in children:
  try:
   if os.waitpid(pid,os.WNOHANG)[0]==0:os.kill(pid,signal.SIGTERM);os.waitpid(pid,0)
  except ChildProcessError:pass
 for channel in channels:
  for fd in channel[:3]:os.close(fd)
 try:run(['stop'])
 finally:run(['disk','rm','--all'])
