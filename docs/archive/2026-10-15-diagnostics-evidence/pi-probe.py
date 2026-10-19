import os,pathlib,subprocess,json,threading,queue,time,signal
BASE=pathlib.Path('/private/tmp/pd-1015');TAG=os.environ.get('PD_TAG','pi-baseline');ROOT=BASE/TAG;ROOT.mkdir(exist_ok=True);BIN=os.environ.get('PD_BIN',str(BASE/'pinfold-baseline'))
env=os.environ.copy()
for k,d in [('XDG_STATE_HOME','s'),('XDG_CONFIG_HOME','c'),('XDG_CACHE_HOME','k')]:env[k]=str(ROOT/d);(ROOT/d/'pinfold').mkdir(parents=True,exist_ok=True)
(ROOT/'s/pinfold/maintenance').touch();project=ROOT/'project';project.mkdir(exist_ok=True)
subprocess.run(['git','init','-q',str(project)],check=True)
config=project/'.pinfold.toml';config.write_text('allow = ["api.github.com"]\ncpus = 1\nmemory = "512M"\n')
evidence={};p=None;name=None
def run(args):
 p=subprocess.run([BIN,*args],env=env,cwd=project,capture_output=True,timeout=30);return {'code':p.returncode,'out':p.stdout.decode(),'err':p.stderr.decode()}
try:
 evidence['trust']=run(['allow']);p=subprocess.Popen([BIN,'pi','--mode','rpc'],env=env,cwd=project,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 q=queue.Queue();err=[]
 def drain():
  for l in p.stdout:q.put(l.decode())
  q.put(None)
 def errors():
  for l in p.stderr:err.append(l.decode())
 threading.Thread(target=drain,daemon=True).start();threading.Thread(target=errors,daemon=True).start()
 p.stdin.write(b'{"type":"get_state","id":"research-ready"}\n');p.stdin.flush()
 lines=[]
 while True:
  l=q.get(timeout=120)
  if l is None:raise RuntimeError('Pi ended before RPC ready: '+''.join(err))
  lines.append(l)
  try:
   v=json.loads(l)
   if v.get('id')=='research-ready':break
  except ValueError:pass
 evidence['rpc-ready']=v
 listing=run(['box','list','--label','dev.pinfold.owner']);boxes=[json.loads(l) for l in listing['out'].splitlines()];box=next(b for b in boxes if b['owner']==p.pid);name=box['name'];evidence['box']=box
 evidence['current-before']=json.loads(run(['config'])['out'])
 evidence['denied-before']=run(['attach','--','curl','-sS','--max-time','5','-w',' HTTP=%{http_code}','http://example.com/'])
 config.write_text('allow = ["example.com"]\ncpus = 1\nmemory = "512M"\n')
 evidence['current-after']=json.loads(run(['config'])['out'])
 evidence['denied-after']=run(['attach','--','curl','-sS','--max-time','5','-w',' HTTP=%{http_code}','http://example.com/'])
 if 'prototype' in TAG:
  runid=box['labels']['dev.pinfold.generation'];evidence['applied-after']=run(['box','inspect-run',runid,'--limit','3']);evidence['explain-after']=run(['box','check-run',runid,'example.com','80','HTTP'])
 else:
  log=ROOT/'s/pinfold/egress'/f'{name}.jsonl';evidence['egress']=[json.loads(l) for l in log.read_text().splitlines()]
 evidence['stderr']=''.join(err)
finally:
 if p and p.poll() is None:
  p.terminate()
  try:p.wait(timeout=40)
  except subprocess.TimeoutExpired:p.kill();p.wait()
 if name:evidence['cleanup']=run(['box','down',name])
 evidence['final']=run(['box','list','--label','dev.pinfold.owner'])
 (BASE/'evidence'/f'{TAG}.json').write_text(json.dumps(evidence,indent=2))
 print(json.dumps(evidence,indent=2))
