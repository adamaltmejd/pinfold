import copy,json,os,pathlib,select,subprocess,sys,tempfile,resource,signal,fcntl,pty,struct,termios,time
P=pathlib.Path(os.environ['P']).resolve(); MODE=sys.argv[1]; BASE=os.environ.get('BASELINE')=='1'
W=pathlib.Path(tempfile.mkdtemp(prefix='pfr.',dir='/tmp')); E=dict(os.environ,XDG_CONFIG_HOME=str(W/'c'),XDG_STATE_HOME=str(W/'s'),PINFOLD_NO_UPDATE_CHECK='1',FIXTURE_TOKEN='synthetic-review-token',PINFOLD_CPUS='2',PINFOLD_MEMORY='1G')
(W/'s/pinfold').mkdir(parents=True); (W/'s/pinfold/maintenance').touch()
name='probe-review-'+W.name[4:]; owners={}; disks=set()
def emit(s): print(str(s).replace(str(W),'$W').replace(str(pathlib.Path.home()),'/home/user'),flush=True)
def run(args,ok=True):
 p=subprocess.run([str(P),*args],env=E,text=True,capture_output=True,timeout=150)
 if ok and p.returncode: raise RuntimeError(p.stdout+p.stderr)
 return p
def limited():
 resource.setrlimit(resource.RLIMIT_FSIZE,(512*1024,512*1024)); signal.signal(signal.SIGXFSZ,signal.SIG_IGN)
def up(s,limit=False):
 err=tempfile.TemporaryFile(mode='w+t'); p=subprocess.Popen([str(P),'box','up'],env=E,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,text=True,preexec_fn=limited if limit else None); owners[s['name']]=(p,err); disks.update(d['name'] for d in s.get('disks',[])); p.stdin.write(json.dumps(s)+'\n'); p.stdin.flush()
 assert select.select([p.stdout],[],[],180)[0], 'readiness deadline'
 row=json.loads(p.stdout.readline()); emit(row if row['event']!='ready' else 'ready '+s['name'])
 if row['event']!='ready': p.stdin.close(); p.wait(timeout=90); err.close(); owners.pop(s['name'])
 return row
def ready(s): assert up(s)['event']=='ready'
def down(s):
 pair=owners.pop(s['name'],None)
 if pair:
  run(['box','down',s['name']],False); p,err=pair; p.stdin.close(); p.wait(timeout=90); err.close()
def x(s,cmd): return run(['box','exec',s['name'],'--','sh','-c','set -eu; '+cmd]).stdout.strip()
def spec(tag,ds): return {'name':name+'-'+tag,'image':'localhost/pinfold/profile-default:latest','cpus':2,'memory':'1G','env':{'HOME':'/home/user'},'disks':ds}
def pi(s): s['egress']={'allow':[],'routes':{'pi':{'login':'pi','provider':'anthropic','from':'FIXTURE_TOKEN','to':'http://127.0.0.1:9'}}}
def terminal(s,missing=False):
 master,slave=pty.openpty(); fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
 def session():
  os.setsid(); fcntl.ioctl(0,termios.TIOCSCTTY,0); os.tcsetpgrp(0,os.getpgrp())
 e=dict(E,TERM='xterm-256color',COLORTERM='truecolor')
 if missing: e.pop('TERM'); e.pop('COLORTERM')
 p=subprocess.Popen([str(P),'box','exec',s['name'],'--tty','--','sh','-c','printf "%s|%s|%s\\n" "$TERM" "$COLORTERM" "$REVIEW_VAR"; stty size; exit 7'],env=e,stdin=slave,stdout=slave,stderr=slave,preexec_fn=session); os.close(slave); out=b''; end=time.monotonic()+60
 try:
  while True:
   assert time.monotonic()<end, 'terminal deadline'
   if select.select([master],[],[],min(1,end-time.monotonic()))[0]:
    try: chunk=os.read(master,65536)
    except OSError: break
    if not chunk: break
    out+=chunk
   elif p.poll() is not None: break
  p.wait(timeout=15); text=out.decode(errors='replace').replace('\r',''); expected='guest-term|guest-color|kept' if missing else 'xterm-256color|truecolor|kept'
  assert p.returncode==7 and expected in text and '36 120' in text, (p.returncode,text)
 finally:
  if p.poll() is None: p.kill(); p.wait()
  os.close(master)
try:
 if MODE=='private':
  s=spec('private',[{'name':name+'-home','guest':'/home/user'}]); pi(s); ready(s); x(s,'chmod 700 "$HOME"; printf kept > "$HOME/kept"'); down(s); row=up(s)
  if BASE and sys.platform=='darwin': assert row['event']=='failed' and any(t in row.get('detail','') for t in ('EACCES','Permission denied')); emit('P2-a reproduced: reused0700 Pi home fails EACCES')
  else: assert row['event']=='ready'; assert x(s,'stat -c %a "$HOME"; cat "$HOME/kept"')=='700\nkept'; down(s); emit('P2-a reused0700 Pi home restart passed; mode/content retained')
  plain=copy.deepcopy(s); plain.pop('egress'); ready(plain); x(plain,'chmod 755 "$HOME"'); down(plain)
 elif MODE=='nested':
  child={'name':name+'-child','guest':'/a/b'}; c=spec('child',[child]); ready(c); x(c,'printf keep > /a/b/sentinel'); down(c)
  f=W/'from'; (f/'b').mkdir(parents=True); (f/'b/sentinel').write_text('overwrite'); (f/'b/added').write_text('leak'); (f/'outer').write_text('parent')
  s=spec('parent',[child,{'name':name+'-parent','guest':'/a','from':str(f)}]); row=up(s)
  if BASE and sys.platform.startswith('linux'): assert row['event']=='failed'; emit('P2-b reproduced startup failure, not overwrite')
  else:
   assert row['event']=='ready'; assert x(s,'cat /a/outer; cat /a/b/sentinel; test ! -e /a/b/added')=='parentkeep'; down(s); emit('P2-b parent fill before reused nested disk passed')
  ready(c); assert x(c,'cat /a/b/sentinel; test ! -e /a/b/added')=='keep'; down(c); emit('nested reused disk content unchanged')
  if not BASE or not sys.platform.startswith('linux'):
   d=copy.deepcopy(s['disks'][1]); d.pop('from'); d['readonly']=True; q=spec('underlay',[d]); ready(q); assert x(q,'cat /a/b/sentinel; cat /a/b/added')=='overwriteleak'; down(q); emit('fresh parent underlay fully populated')
  h=W/'mount'; h.mkdir(); (h/'caller').write_text('host'); s=spec('mount',[{'name':name+'-mounted','guest':'/a','from':str(f)}]); s['mounts']=[{'host':str(h),'guest':'/a/b'}]; row=up(s)
  if BASE and sys.platform.startswith('linux'): assert row['event']=='failed'
  else: assert row['event']=='ready'; assert x(s,'cat /a/outer; cat /a/b/caller')=='parenthost'; down(s)
  assert sorted(p.name for p in h.iterdir())==['caller']
  if not BASE or not sys.platform.startswith('linux'):
   d=copy.deepcopy(s['disks'][0]); d.pop('from'); d['readonly']=True; q=spec('mount-underlay',[d]); ready(q); assert x(q,'cat /a/b/sentinel; cat /a/b/added')=='overwriteleak'; down(q)
  emit('nested caller mount '+('baseline fails' if BASE and sys.platform.startswith('linux') else 'passed')+'; host content unchanged')
 elif MODE=='terminal':
  s=spec('terminal',[{'name':name+'-home','guest':'/home/user'}]); pi(s); s['env'].update(TERM='guest-term',COLORTERM='guest-color',REVIEW_VAR='kept'); ready(s); assert x(s,'printf \"%s|%s|%s\" \"$TERM\" \"$COLORTERM\" \"$REVIEW_VAR\"')=='guest-term|guest-color|kept'; terminal(s); terminal(s,True); down(s); emit('P3-1 persistent env, nonTTY/TTY TERM COLORTERM, window and exit7 passed')
 elif MODE=='normalized':
  f=W/'from'; f.mkdir(); (f/'value').write_text('normalized'); s=spec('normalized',[{'name':name+'-normalized','guest':'/alias/../a','from':str(f)}]); row=up(s)
  if BASE and sys.platform.startswith('linux'): assert row['event']=='failed' and 'must be plain' in row.get('detail',''); emit('P3-4 raw fill destination rejected despite normalized attachment')
  else: assert row['event']=='ready'; assert x(s,'cat /a/value')=='normalized'; down(s); emit('P3-4 normalized fill destination passed')
 elif MODE=='multifresh':
  a=W/'from-a'; b=W/'from-b'; (a/'b').mkdir(parents=True); b.mkdir(); (a/'outer').write_text('parent'); (a/'b/hidden').write_text('underlay'); (b/'child').write_text('child'); s=spec('multi',[{'name':name+'-b','guest':'/a/b','from':str(b)},{'name':name+'-a','guest':'/a','from':str(a)}]); ready(s); assert x(s,'cat /a/outer; cat /a/b/child')=='parentchild'; down(s); s['disks']=s['disks'][1:]; ready(s); assert x(s,'cat /a/b/hidden')=='underlay'; down(s); emit('reversed nested fresh fills keep metadata and stream order')
 elif MODE=='rollback':
  s=spec('rollback',[{'name':name+'-home','guest':'/home/user'}]); ready(s); x(s,'printf keep > "$HOME/kept"'); down(s); bad=copy.deepcopy(s); pi(bad); row=up(bad,True); assert row['event']=='failed' and 'File too large' in row.get('detail','')
  ro=copy.deepcopy(s); ro['disks'][0]['readonly']=True; row=up(ro); assert row['event']=='refused' and row['reason']=='disk-unclean'; emit('P3-3 pre-BOOT failed preparation retains unclean marker')
  ready(s); assert x(s,'cat "$HOME/kept"')=='keep'; down(s)
 else: raise RuntimeError('unknown mode')
finally:
 for n in list(owners): down({'name':n})
 existing={json.loads(l)['name'] for l in run(['disk','list','--json']).stdout.splitlines() if l}
 for disk in disks&existing: run(['disk','rm',disk])
 emit('owned boxes/disks removed')
