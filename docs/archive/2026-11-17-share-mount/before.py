import json,os,pathlib,shutil,subprocess,tempfile
r=pathlib.Path(tempfile.mkdtemp(prefix='pf279f.',dir='/tmp')).resolve();b=pathlib.Path('target/debug/pinfold').resolve();e=dict(os.environ,XDG_CONFIG_HOME=str(r/'c'),XDG_STATE_HOME=str(r/'s'),PINFOLD_NO_UPDATE_CHECK='1');stamp=r/'s/pinfold/maintenance';stamp.parent.mkdir(parents=True);stamp.touch();s=r/'share';o=r/'other';(s/'.git/hooks').mkdir(parents=True);o.mkdir();(s/'.git/protected').write_text('protected\n');p=None;name='probe-share279-fixes-before'
try:
 print('before Git names:',sorted(x.name for x in (s/'.git').iterdir()),flush=True)
 spec=dict(name=name,image='localhost/pinfold/profile-default',mounts=[dict(host=str(s),guest='/project',kind='share'),dict(host=str(o),guest='/project/.GIT/hooks',kind='readonly')],env={'HOME':'/tmp'},cpus=2,memory='1G')
 p=subprocess.Popen([str(b),'box','up'],env=e,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);p.stdin.write(json.dumps(spec)+'\n');p.stdin.flush();print(p.stdout.readline().replace(str(r),'$W').strip(),flush=True)
 print('after Git names:',sorted(x.name for x in (s/'.git').iterdir()),flush=True)
 print('protected bytes:',(s/'.git/protected').read_text().strip(),flush=True)
finally:
 q=subprocess.run([str(b),'box','down',name],env=e,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);print('down rc=',q.returncode,flush=True)
 if p:p.stdin.close();p.wait(timeout=90)
 shutil.rmtree(r);print('fixture removed=',not r.exists(),flush=True)
