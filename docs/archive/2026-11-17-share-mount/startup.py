import json,os,pathlib,subprocess,tempfile,shutil,time
r=pathlib.Path(tempfile.mkdtemp(prefix='pf279f.',dir='/tmp')).resolve();b=pathlib.Path('target/debug/pinfold').resolve();e=dict(os.environ,XDG_CONFIG_HOME=str(r/'c'),XDG_STATE_HOME=str(r/'s'),PINFOLD_NO_UPDATE_CHECK='1');stamp=r/'s/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch();src=r/'source';src.mkdir();(src/'zeros').open('wb').truncate(512*1024*1024);phase=os.environ.get('PF279_PHASE','before');name='probe-share279-startup-'+phase;disk=name+'-disk';p=None
into=r/'into';into.mkdir()
ge=dict(e,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1')
subprocess.run(['git','init','-q','--initial-branch=main',str(into)],env=ge,check=True)
try:
 spec=dict(name=name,image='localhost/pinfold/profile-default',disks=[dict(name=disk,guest='/work',size='1G',**{'from':str(src)})],env={'HOME':'/tmp'},cpus=2,memory='1G')
 p=subprocess.Popen([str(b),'box','up'],env=e,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);p.stdin.write(json.dumps(spec)+'\n');p.stdin.flush();deadline=time.monotonic()+120;found=False
 while time.monotonic()<deadline and p.poll() is None:
  q=subprocess.run(['container','inspect',name],capture_output=True,text=True)
  if q.returncode==0:
   items=json.loads(q.stdout)
   if items and items[0].get('status',{}).get('state')=='running' and 'pinfold-disk' in json.dumps(items[0]):found=True;break
 print('preparation-helper-observed=',found,flush=True)
 if found:
  q=subprocess.run([str(b),'box','exec',name,'--','/usr/bin/id','-u'],env=e,capture_output=True,text=True);print('early exec rc=',q.returncode,'stdout=',q.stdout.strip(),'stderr=',q.stderr.strip(),flush=True)
  assert q.returncode==(3 if phase=='after' else 0)
  if phase=='after':
   q=subprocess.run([str(b),'box','fetch',name,'--repo','/work/repo','--into',str(into)],env=ge,capture_output=True,text=True);print('early fetch rc=',q.returncode,'stderr=',q.stderr.strip(),flush=True);assert q.returncode==3
 print('up:',p.stdout.readline().replace(str(r),'$W').strip(),flush=True)
 if phase=='after':
  q=subprocess.run([str(b),'box','exec',name,'--','/bin/sh','-c','id -u; git init -q --initial-branch=main /work/repo; git -c user.name=user -c user.email=user@example.invalid -C /work/repo commit --allow-empty -qm fixture'],env=e,capture_output=True,text=True);print('ready exec rc=',q.returncode,'uid=',q.stdout.strip(),flush=True);assert q.returncode==0 and q.stdout.strip()==str(os.getuid())
  q=subprocess.run([str(b),'box','fetch',name,'--repo','/work/repo','--into',str(into)],env=ge,capture_output=True,text=True);print('ready fetch rc=',q.returncode,q.stdout.strip(),flush=True);assert q.returncode==0
finally:
 q=subprocess.run([str(b),'box','down',name],env=e,capture_output=True,text=True);print('down rc=',q.returncode,flush=True)
 if p:p.stdin.close();p.wait(timeout=90)
 q=subprocess.run([str(b),'disk','rm',disk,'--json'],env=e,capture_output=True,text=True);print('disk cleanup:',q.returncode,q.stdout.strip(),flush=True);shutil.rmtree(r);print('fixture removed=',not r.exists(),flush=True)
