from pathlib import Path
exec(Path('/tmp/pd-1015/probe.py').read_text().split('\ntry:\n s=spec(')[0])
import selectors
try:
 s=spec('pd-edge');s['egress']={'allow':['api.github.com'],'routes':{'ok.internal':f'127.0.0.1:{server.server_port}'}}
 o=Owner(s);record('ready',o.first);rid=o.first['run_id']
 record('connect-positive',run(['box','exec',o.name,'--','curl','-sS','--max-time','15','-o','/dev/null','-w','%{http_code}','https://api.github.com/']))
 record('many-decisions',run(['box','exec',o.name,'--','/bin/sh','-c','i=0; while [ "$i" -lt 120 ]; do curl -sS --max-time 3 -o /dev/null http://ok.internal/ok || exit; i=$((i+1)); done']))
 import fcntl
 rfd,wfd=os.pipe();fcntl.fcntl(wfd,fcntl.F_SETPIPE_SZ,4096)
 observer=subprocess.Popen([BIN,'box','inspect-run',rid,'--limit','100'],env=env,stdout=wfd,stderr=subprocess.PIPE)
 os.close(wfd);observer.stdout=os.fdopen(rfd,'rb')
 selector=selectors.DefaultSelector();selector.register(observer.stdout,selectors.EVENT_READ);events=selector.select(3);selector.close()
 try:observer.wait(timeout=0.2);blocked=False
 except subprocess.TimeoutExpired:blocked=True
 record('slow-observer',{'readable':bool(events),'still_waiting_after_200ms':blocked})
 record('owner-during-slow-observer',run(['box','exec',o.name,'--','/bin/sh','-c','printf still-alive']))
 if observer.poll() is None:observer.terminate()
 out,err=observer.communicate(timeout=5);record('cancelled-query',{'code':observer.returncode,'captured_bytes':len(out)})
 record('fresh-bounded-query',run(['box','inspect-run',rid,'--limit','3']))
 killed=subprocess.run(['podman','kill','--signal','KILL',o.name],capture_output=True,timeout=10);record('runtime-kill',{'code':killed.returncode,'stderr':killed.stderr.decode()})
 o.p.wait(timeout=30);record('unexpected-runtime-end',{'events':o.events,'code':o.p.returncode});record('unexpected-postmortem',run(['box','inspect-run',rid,'--limit','3']))
finally:
 for o in owners:
  if o.p.poll() is None:o.p.terminate();o.p.wait(timeout=40)
 for n in set(names):evidence.setdefault('cleanup',{})[n]=run(['box','down',n],timeout=40)
 server.shutdown();server.server_close();reserved.close()
 evidence['final_inventory']=run(['box','list','--label','dev.pinfold.owner'])
 (BASE/'evidence'/f'{TAG}.json').write_text(json.dumps(evidence,indent=2))
